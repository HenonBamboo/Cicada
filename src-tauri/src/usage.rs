//! 服务商实际返回的 Token 用量。快照不累加，缺失用量不估算。

use std::collections::BTreeMap;
use std::sync::Mutex;

use chrono::{Datelike, Days};
use rusqlite::{Connection, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::clock::{Clock, parse_cutoff};
use crate::config::AppConfig;
use crate::error::AppResult;
use crate::llm::Endpoint;
use crate::service::{DATE_FMT, DATETIME_FMT};

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

impl TokenUsage {
    pub fn complete(self) -> bool {
        self.input_tokens.is_some() && self.output_tokens.is_some() && self.total_tokens.is_some()
    }
}

#[derive(Clone, Copy)]
pub struct UsageContext<'a> {
    pub db: &'a Mutex<Connection>,
    pub clock: &'a dyn Clock,
    pub config: &'a AppConfig,
    pub endpoint: &'a Endpoint,
    pub purpose: &'a str,
    pub request_id: &'a str,
    pub notify: Option<&'a tokio::sync::Notify>,
}

pub struct UsageAttempt<'a> {
    context: UsageContext<'a>,
    started_at: String,
    business_date: Option<String>,
    usage: Mutex<TokenUsage>,
    status: &'static str,
}

impl<'a> UsageContext<'a> {
    pub fn begin(self) -> UsageAttempt<'a> {
        let now = self.clock.now();
        let business_date = parse_cutoff(&self.config.time.day_cutoff)
            .ok()
            .map(|cutoff| {
                crate::clock::business_date(now, cutoff)
                    .format(DATE_FMT)
                    .to_string()
            });
        UsageAttempt {
            context: self,
            started_at: now.format(DATETIME_FMT).to_string(),
            business_date,
            usage: Mutex::new(TokenUsage::default()),
            status: "cancelled",
        }
    }
}

impl UsageAttempt<'_> {
    /// 接收适配层组装的完整快照，缺失字段不能沿用上一个快照的旧值。
    pub fn observe(&self, snapshot: TokenUsage) {
        if let Ok(mut usage) = self.usage.lock() {
            let valid = |v: Option<u64>| v.filter(|n| *n <= i64::MAX as u64);
            *usage = TokenUsage {
                input_tokens: valid(snapshot.input_tokens),
                output_tokens: valid(snapshot.output_tokens),
                total_tokens: valid(snapshot.total_tokens),
            };
        }
    }

    pub fn completed(&mut self, success: bool) {
        self.status = if success { "success" } else { "failed" };
    }

    fn save(&self) -> AppResult<bool> {
        let Some(date) = &self.business_date else {
            return Ok(false);
        };
        let origin = crate::secrets::endpoint_origin(self.context.endpoint)?;
        let origin_hash = format!("{:x}", Sha256::digest(origin.as_bytes()));
        let Ok(usage) = self.usage.lock().map(|usage| *usage) else {
            return Ok(false);
        };
        let Ok(conn) = self.context.db.lock() else {
            return Ok(false);
        };
        conn.execute(
            "INSERT INTO token_usage(id,started_at,business_date,provider,origin_hash,model,purpose,request_id,status,input_tokens,output_tokens,total_tokens)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![uuid::Uuid::new_v4().to_string(), self.started_at, date,
                self.context.endpoint.provider.as_str(), origin_hash, self.context.endpoint.model,
                self.context.purpose, self.context.request_id, self.status,
                usage.input_tokens.map(|v| v as i64), usage.output_tokens.map(|v| v as i64),
                usage.total_tokens.map(|v| v as i64)],
        )?;
        Ok(true)
    }
}

impl Drop for UsageAttempt<'_> {
    fn drop(&mut self) {
        // 统计失败不改变模型结果；save 返回时已释放数据库锁。
        match self.save() {
            Ok(true) => {
                if let Some(notify) = self.context.notify {
                    notify.notify_one();
                }
            }
            // 不输出请求、服务地址、凭据或底层错误正文。
            _ => eprintln!("知了：本次模型用量未能写入本地统计。"),
        }
    }
}

#[derive(Default, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageGroup {
    pub provider: String,
    pub model: String,
    pub purpose: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub requests: u64,
    pub unknown_requests: u64,
}

impl UsageGroup {
    fn add(&mut self, usage: TokenUsage) {
        self.requests = self.requests.saturating_add(1);
        if let (Some(input), Some(output), Some(total)) =
            (usage.input_tokens, usage.output_tokens, usage.total_tokens)
        {
            self.input_tokens = self.input_tokens.saturating_add(input);
            self.output_tokens = self.output_tokens.saturating_add(output);
            self.total_tokens = self.total_tokens.saturating_add(total);
        } else {
            self.unknown_requests = self.unknown_requests.saturating_add(1);
        }
    }
}

#[derive(Default, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsagePeriod {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub requests: u64,
    pub unknown_requests: u64,
    pub by_model: Vec<UsageGroup>,
    pub by_purpose: Vec<UsageGroup>,
}

#[derive(Default, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub today: UsagePeriod,
    pub month: UsagePeriod,
    pub business_date: String,
    pub daily: Vec<UsageDay>,
}

#[derive(Default, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageDay {
    pub date: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub requests: u64,
    pub unknown_requests: u64,
}

#[derive(Default)]
struct Aggregate {
    total: UsageGroup,
    model: BTreeMap<(String, String), UsageGroup>,
    purpose: BTreeMap<String, UsageGroup>,
}

impl Aggregate {
    fn add(&mut self, provider: &str, model: &str, purpose: &str, usage: TokenUsage) {
        self.total.add(usage);
        self.model
            .entry((provider.into(), model.into()))
            .or_insert_with(|| UsageGroup {
                provider: provider.into(),
                model: model.into(),
                ..Default::default()
            })
            .add(usage);
        self.purpose
            .entry(purpose.into())
            .or_insert_with(|| UsageGroup {
                purpose: purpose.into(),
                ..Default::default()
            })
            .add(usage);
    }

    fn finish(self) -> UsagePeriod {
        UsagePeriod {
            input_tokens: self.total.input_tokens,
            output_tokens: self.total.output_tokens,
            total_tokens: self.total.total_tokens,
            requests: self.total.requests,
            unknown_requests: self.total.unknown_requests,
            by_model: self.model.into_values().collect(),
            by_purpose: self.purpose.into_values().collect(),
        }
    }
}

pub fn query(conn: &Connection, clock: &dyn Clock, config: &AppConfig) -> AppResult<UsageSummary> {
    let today = clock.business_date(parse_cutoff(&config.time.day_cutoff)?);
    let heat_from = today
        .checked_sub_days(Days::new(366))
        .unwrap_or(today)
        .format(DATE_FMT)
        .to_string();
    let first = today
        .with_day(1)
        .unwrap_or(today)
        .format(DATE_FMT)
        .to_string();
    let today = today.format(DATE_FMT).to_string();
    let mut stmt = conn.prepare(
        "SELECT business_date,provider,model,purpose,input_tokens,output_tokens,total_tokens,status
         FROM token_usage WHERE business_date >= ?1 AND business_date <= ?2",
    )?;
    let rows = stmt.query_map(params![first, today], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            TokenUsage {
                input_tokens: row.get::<_, Option<i64>>(4)?.map(|v| v as u64),
                output_tokens: row.get::<_, Option<i64>>(5)?.map(|v| v as u64),
                total_tokens: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
            },
            row.get::<_, String>(7)?,
        ))
    })?;
    let mut day = Aggregate::default();
    let mut month = Aggregate::default();
    for row in rows {
        let (date, provider, model, purpose, usage, status) = row?;
        // 失败/取消的快照可能只是累计到一半，不能当作最终用量。
        let usage = if status == "success" {
            usage
        } else {
            TokenUsage::default()
        };
        month.add(&provider, &model, &purpose, usage);
        if date == today {
            day.add(&provider, &model, &purpose, usage);
        }
    }
    // 日历只传每日汇总，不把全年逐次请求明细带到前端；未知用量单独计数。
    let mut stmt = conn.prepare(
        "WITH daily_source AS (
           SELECT *, (status='success' AND input_tokens IS NOT NULL
             AND output_tokens IS NOT NULL AND total_tokens IS NOT NULL) AS known
           FROM token_usage WHERE business_date > ?1 AND business_date <= ?2
         )
         SELECT business_date,
           SUM(CASE WHEN known THEN input_tokens ELSE 0 END),
           SUM(CASE WHEN known THEN output_tokens ELSE 0 END),
           SUM(CASE WHEN known THEN total_tokens ELSE 0 END), COUNT(*), SUM(NOT known)
         FROM daily_source GROUP BY business_date ORDER BY business_date",
    )?;
    let daily = stmt
        .query_map(params![heat_from, today], |row| {
            Ok(UsageDay {
                date: row.get(0)?,
                input_tokens: row.get(1)?,
                output_tokens: row.get(2)?,
                total_tokens: row.get(3)?,
                requests: row.get(4)?,
                unknown_requests: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(UsageSummary {
        today: day.finish(),
        month: month.finish(),
        business_date: today,
        daily,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ProviderId;
    use crate::service::testing::{clock, db};

    fn endpoint() -> Endpoint {
        Endpoint {
            provider: ProviderId::Custom,
            base_url: "http://EXAMPLE.com:80/v1?secret=never-store".into(),
            model: "test-model".into(),
        }
    }

    fn context<'a>(
        db: &'a Mutex<Connection>,
        clock: &'a dyn Clock,
        config: &'a AppConfig,
        endpoint: &'a Endpoint,
    ) -> UsageContext<'a> {
        UsageContext {
            db,
            clock,
            config,
            endpoint,
            purpose: "conversation",
            request_id: "request-1",
            notify: None,
        }
    }

    #[test]
    fn 用量快照替换不相加且取消不冒充最终用量() {
        let conn = Mutex::new(db());
        let time = clock("2026-10-21 10:00:00");
        let config = AppConfig::default();
        let ep = endpoint();
        {
            let mut attempt = context(&conn, &time, &config, &ep).begin();
            attempt.observe(TokenUsage {
                input_tokens: Some(10),
                output_tokens: Some(1),
                total_tokens: Some(11),
            });
            attempt.observe(TokenUsage {
                input_tokens: Some(10),
                output_tokens: Some(4),
                total_tokens: Some(14),
            });
            attempt.completed(true);
        }
        {
            let attempt = context(&conn, &time, &config, &ep).begin();
            attempt.observe(TokenUsage {
                input_tokens: Some(10),
                output_tokens: Some(2),
                total_tokens: Some(12),
            });
        }
        drop(context(&conn, &time, &config, &ep).begin());
        {
            let mut attempt = context(&conn, &time, &config, &ep).begin();
            attempt.observe(TokenUsage {
                input_tokens: Some(10),
                output_tokens: Some(1),
                total_tokens: Some(11),
            });
            attempt.observe(TokenUsage {
                input_tokens: None,
                output_tokens: Some(4),
                total_tokens: None,
            });
            attempt.completed(true);
        }
        let conn = conn.lock().unwrap();
        let summary = query(&conn, &time, &config).unwrap();
        assert_eq!(summary.today.requests, 4);
        assert_eq!(summary.today.unknown_requests, 3);
        assert_eq!(summary.today.input_tokens, 10);
        assert_eq!(summary.today.output_tokens, 4);
        assert_eq!(summary.today.total_tokens, 14);
        assert_eq!(summary.daily.len(), 1);
        assert_eq!(summary.daily[0].total_tokens, 14);
        assert_eq!(summary.daily[0].unknown_requests, 3);
        let (origin, tokens): (String, Option<i64>) = conn.query_row(
            "SELECT origin_hash,total_tokens FROM token_usage WHERE status='cancelled' AND total_tokens IS NOT NULL", [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(
            origin,
            format!("{:x}", Sha256::digest(b"http://example.com"))
        );
        assert_eq!(tokens, Some(12));
    }

    #[test]
    fn 用量按业务日和业务月份聚合并排除未来数据() {
        let conn = Mutex::new(db());
        let mut config = AppConfig::default();
        config.time.day_cutoff = "04:00".into();
        let ep = endpoint();
        for date in [
            "2025-10-01 10:00:00",
            "2026-09-30 10:00:00",
            "2026-10-01 10:00:00",
            "2026-11-01 02:00:00",
            "2026-11-01 05:00:00",
        ] {
            let time = clock(date);
            let mut attempt = context(&conn, &time, &config, &ep).begin();
            attempt.observe(TokenUsage {
                input_tokens: Some(2),
                output_tokens: Some(3),
                total_tokens: Some(5),
            });
            attempt.completed(true);
        }
        let time = clock("2026-11-01 03:00:00");
        let summary = query(&conn.lock().unwrap(), &time, &config).unwrap();
        assert_eq!(summary.today.requests, 1);
        assert_eq!(summary.month.requests, 2);
        assert_eq!(summary.month.total_tokens, 10);
        assert_eq!(summary.month.by_model.len(), 1);
        assert_eq!(summary.month.by_model[0].purpose, "");
        assert_eq!(summary.month.by_purpose[0].provider, "");
        assert_eq!(summary.business_date, "2026-10-31");
        assert_eq!(summary.daily.len(), 3);
        assert_eq!(summary.daily[0].date, "2026-09-30");
        assert_eq!(
            summary
                .daily
                .iter()
                .map(|day| day.total_tokens)
                .sum::<u64>(),
            15
        );
    }
}
