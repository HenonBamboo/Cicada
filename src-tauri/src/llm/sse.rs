//! Server-Sent Events 解析：把字节流切成一条条 data

use futures_util::StreamExt;

use super::LlmError;

/// 读取流式响应，逐条交给 handle。字节块可能截断多字节汉字，这里按 UTF-8 边界拼接
pub async fn for_each_event(
    resp: reqwest::Response,
    mut handle: impl FnMut(&str) -> Result<(), LlmError>,
) -> Result<(), LlmError> {
    let mut parser = SseParser::default();
    let mut pending: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        pending.extend_from_slice(&chunk?);
        let valid = match std::str::from_utf8(&pending) {
            Ok(_) => pending.len(),
            Err(e) if e.error_len().is_some() => {
                return Err(LlmError::BadResponse("回复包含无效的 UTF-8 字节".into()));
            }
            Err(e) => e.valid_up_to(),
        };
        let text = String::from_utf8_lossy(&pending[..valid]).into_owned();
        pending.drain(..valid);
        for data in parser.feed(&text) {
            handle(&data)?;
        }
    }
    if !pending.is_empty() {
        return Err(LlmError::Network("回复中断，文字未接收完整".into()));
    }
    if let Some(data) = parser.finish() {
        handle(&data)?;
    }
    Ok(())
}

/// 增量解析器：喂入任意切分的字节块，取出完整事件的 data 内容
#[derive(Default)]
pub struct SseParser {
    buf: String,
    data: Vec<String>,
}

impl SseParser {
    /// 返回本次喂入后完成的事件（多行 data 用换行连接）
    pub fn feed(&mut self, chunk: &str) -> Vec<String> {
        self.buf.push_str(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.find('\n') {
            let line: String = self.buf.drain(..=pos).collect();
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    out.push(self.data.join("\n"));
                    self.data.clear();
                }
            } else if let Some(d) = line.strip_prefix("data:") {
                self.data.push(d.strip_prefix(' ').unwrap_or(d).to_string());
            }
            // event: / id: / 注释行不需要：各协议的事件类型都在 data 的 JSON 里
        }
        out
    }

    /// 流结束时取出最后一个没有空行收尾的事件
    pub fn finish(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.buf);
        let rest = rest.trim_end_matches(['\n', '\r']);
        if let Some(d) = rest.strip_prefix("data:") {
            self.data.push(d.trim_start().to_string());
        }
        (!self.data.is_empty()).then(|| {
            let s = self.data.join("\n");
            self.data.clear();
            s
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 任意切分都能拼出完整事件() {
        let mut p = SseParser::default();
        let mut got = p.feed("event: x\ndata: {\"a\"");
        assert!(got.is_empty());
        got.extend(p.feed(":1}\n\ndata: [DONE]\r\n"));
        got.extend(p.feed("\r\n"));
        assert_eq!(got, ["{\"a\":1}", "[DONE]"]);
    }

    #[test]
    fn 结尾没有空行也能取出() {
        let mut p = SseParser::default();
        assert!(p.feed("data: last").is_empty());
        assert_eq!(p.finish().as_deref(), Some("last"));
    }
}
