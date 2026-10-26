import { useEffect, useRef, useState } from "react";
import {
  getTokenUsage,
  onConfigChanged,
  onDataChanged,
  onTokenUsageChanged,
  type UsageGroup,
  type UsageSummary,
} from "../lib/commands";
import { Seg } from "./SettingsControls";
import { UsageHeatmap } from "./UsageHeatmap";

const number = new Intl.NumberFormat("zh-CN");
const PURPOSES: Record<string, string> = {
  chat: "对话",
  vision: "识图",
  ui_copy: "问候语与页面文案",
  report: "报告",
  prompt_suggestion: "提示词优化",
  connection_test: "连接测试",
};
const PROVIDERS: Record<string, string> = {
  deepseek: "DeepSeek",
  qwen: "通义千问",
  kimi: "Kimi",
  zhipu: "智谱 GLM",
  claude: "Claude",
  custom: "自定义服务",
};

function groupName(group: UsageGroup, dimension: "model" | "purpose") {
  return dimension === "purpose"
    ? PURPOSES[group.purpose] || group.purpose
    : `${PROVIDERS[group.provider] || group.provider} · ${group.model}`;
}

export function TokenUsage() {
  const [usage, setUsage] = useState<UsageSummary | null>(null);
  const [period, setPeriod] = useState<"today" | "month">("today");
  const [dimension, setDimension] = useState<"model" | "purpose">("model");
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [listenerError, setListenerError] = useState("");
  const refresh = useRef<() => void>(() => {});

  useEffect(() => {
    let alive = true;
    let request = 0;
    const stops: (() => void)[] = [];
    const load = async () => {
      const current = ++request;
      if (!alive) return;
      setBusy(true);
      setError("");
      try {
        const value = await getTokenUsage();
        if (alive && current === request) setUsage(value);
      } catch (cause) {
        if (alive && current === request) setError(`用量读取失败：${String(cause)}`);
      } finally {
        if (alive && current === request) setBusy(false);
      }
    };
    const reload = () => void load();
    refresh.current = reload;
    const register = async (subscription: Promise<() => void>) => {
      const stop = await subscription;
      if (alive) stops.push(stop);
      else stop();
    };
    // 先注册再读取，避免首读期间完成的请求未反映到页面。
    void Promise.allSettled([
      register(onTokenUsageChanged(reload)),
      register(onConfigChanged(reload)),
      register(onDataChanged(reload)),
    ]).then((results) => {
      if (!alive) return;
      if (results.some((result) => result.status === "rejected")) {
        setListenerError("自动刷新暂不可用，请手动刷新用量。");
      }
      void load();
    });
    return () => {
      alive = false;
      refresh.current = () => {};
      stops.forEach((stop) => stop());
    };
  }, []);

  const current = usage?.[period];
  const groups = current?.[dimension === "model" ? "byModel" : "byPurpose"] ?? [];
  return (
    <div className="card settings-group token-usage">
      <div className="settings-row">
        <div className="lab">
          <b>Token 用量</b>
          <small>服务商返回的已知用量</small>
        </div>
        <Seg
          value={period}
          choices={[
            ["today", "今日"],
            ["month", "本月"],
          ]}
          onChange={setPeriod}
        />
        <button className="btn" disabled={busy} onClick={() => refresh.current()}>
          {busy ? "读取中…" : "刷新"}
        </button>
      </div>
      {(error || listenerError) && (
        <p className="settings-note" role="status">
          {error || listenerError}
        </p>
      )}
      {current && (
        <>
          <dl className="token-usage-totals">
            {[
              ["输入", current.inputTokens],
              ["输出", current.outputTokens],
              ["合计", current.totalTokens],
            ].map(([label, value]) => (
              <div key={label}>
                <dt>{label}</dt>
                <dd>{number.format(Number(value))}</dd>
              </div>
            ))}
          </dl>
          <p className="token-usage-count">
            共 {number.format(current.requests)} 次请求
            {current.unknownRequests > 0 &&
              `，其中 ${number.format(current.unknownRequests)} 次用量未知`}
          </p>
          {current.requests > 0 && (
            <details className="token-usage-details">
              <summary>按模型和用途查看</summary>
              <Seg
                value={dimension}
                choices={[
                  ["model", "按模型"],
                  ["purpose", "按用途"],
                ]}
                onChange={setDimension}
              />
              <div className="token-usage-table">
                <table>
                  <thead>
                    <tr>
                      <th scope="col">{dimension === "model" ? "模型" : "用途"}</th>
                      <th scope="col">输入</th>
                      <th scope="col">输出</th>
                      <th scope="col">合计</th>
                    </tr>
                  </thead>
                  <tbody>
                    {groups.map((group) => (
                      <tr key={JSON.stringify([group.provider, group.model, group.purpose])}>
                        <th scope="row">
                          {groupName(group, dimension)}
                          <small>
                            {number.format(group.requests)} 次请求
                            {group.unknownRequests > 0 &&
                              ` · ${number.format(group.unknownRequests)} 次用量未知`}
                          </small>
                        </th>
                        <td>{number.format(group.inputTokens)}</td>
                        <td>{number.format(group.outputTokens)}</td>
                        <td>{number.format(group.totalTokens)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </details>
          )}
        </>
      )}
      {usage && <UsageHeatmap today={usage.businessDate} days={usage.daily} />}
      <p className="settings-note">
        仅统计本机知了的模型调用，从 0.2.5 开始记录，不代表账户账单或余额。
      </p>
    </div>
  );
}
