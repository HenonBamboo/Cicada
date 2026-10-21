import { useEffect, useState, type ReactNode } from "react";
import {
  clearApiKey,
  createBackup,
  checkUpdate,
  getUpdateStatus,
  getModelSettings,
  listBackups,
  openFolder,
  onUpdateStatus,
  restoreBackup,
  saveModelSettings,
  testModel,
  type UpdateStatusView,
} from "../lib/commands";
import type { BackupEntry, Endpoint, ModelSettings, Provider } from "../lib/types";
import "./settings.css";
import { getPreferences, savePreferences, type Preferences } from "../lib/settings";
import { getBootstrap, onConfigChanged, setAppearance } from "../lib/commands";
import type { Appearance } from "../lib/types";
import { PreferenceSections } from "./PreferenceSections";
import { PromptSettings } from "./PromptSettings";
import { DataExchange } from "./DataExchange";
import packageInfo from "../../package.json";

const PROVIDERS: [Provider, string, string][] = [
  ["deepseek", "DeepSeek", "https://api.deepseek.com/v1"],
  ["qwen", "通义千问", "https://dashscope.aliyuncs.com/compatible-mode/v1"],
  ["kimi", "Kimi（月之暗面）", "https://api.moonshot.cn/v1"],
  ["zhipu", "智谱 GLM", "https://open.bigmodel.cn/api/paas/v4"],
  ["claude", "Claude", "https://api.anthropic.com/v1"],
  ["custom", "自定义（OpenAI 兼容）", ""],
];
function sameCredentialScope(a: Endpoint, b: Endpoint) {
  if (a.provider !== b.provider) return false;
  try {
    return new URL(a.baseUrl.trim()).origin === new URL(b.baseUrl.trim()).origin;
  } catch {
    return false;
  }
}
function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="settings-row">
      <div className="lab">
        <b>{label}</b>
        {hint && <small>{hint}</small>}
      </div>
      {children}
    </div>
  );
}
function EndpointFields({
  value,
  onChange,
  secret,
  onSecret,
  onClear,
  vision = false,
}: {
  value: Endpoint & { hasKey?: boolean };
  onChange: (value: Endpoint) => void;
  secret: string;
  onSecret: (value: string) => void;
  onClear: () => void;
  vision?: boolean;
}) {
  return (
    <>
      <Row label={vision ? "识图服务商" : "服务商"} hint="兼容 OpenAI 格式的接口都可以接入">
        <select
          className="field wide"
          value={value.provider}
          onChange={(e) => {
            const provider = e.target.value as Provider;
            onChange({
              provider,
              baseUrl: PROVIDERS.find((p) => p[0] === provider)![2],
              model: "",
            });
            onSecret("");
          }}
        >
          {PROVIDERS.map(([id, label]) => (
            <option key={id} value={id}>
              {label}
            </option>
          ))}
        </select>
      </Row>
      <Row label="Base URL">
        <input
          aria-label={vision ? "识图 Base URL" : "Base URL"}
          className="field wide"
          value={value.baseUrl}
          onChange={(e) => onChange({ ...value, baseUrl: e.target.value })}
        />
      </Row>
      <Row label="API Key" hint="按服务商和接口地址保存在 Windows 凭据管理器">
        <div className="settings-secret">
          <input
            aria-label={vision ? "识图 API Key" : "API Key"}
            className="field"
            type="password"
            autoComplete="off"
            value={secret}
            placeholder={value.hasKey ? "已保存，留空保持不变" : "填写服务商 API Key"}
            onChange={(e) => onSecret(e.target.value)}
          />
          {value.hasKey && (
            <button
              className="btn"
              type="button"
              aria-label={vision ? "清除识图 API Key" : "清除 API Key"}
              onClick={onClear}
            >
              清除
            </button>
          )}
        </div>
      </Row>
      <Row
        label={vision ? "识图模型名称" : "对话模型"}
        hint={vision ? "填写支持图片输入的模型名称" : "需要支持工具调用"}
      >
        <input
          aria-label={vision ? "识图模型名称" : "对话模型"}
          className="field wide"
          value={value.model}
          onChange={(e) => onChange({ ...value, model: e.target.value })}
        />
      </Row>
    </>
  );
}
export function Settings({
  dataDir,
  onBack,
  onModelSaved,
  onboarding = false,
  onInstallUpdate,
  onClearUpdateError,
  updating = false,
  updateError = null,
}: {
  dataDir: string;
  onBack: () => void;
  onModelSaved?: () => void;
  onboarding?: boolean;
  onInstallUpdate?: () => void;
  onClearUpdateError?: () => void;
  updating?: boolean;
  updateError?: string | null;
}) {
  const [preferences, setPreferences] = useState<Preferences | null>(null);
  const [appearance, setLook] = useState<Appearance | null>(null);
  const [glassSupported, setGlassSupported] = useState(false);
  const [active, setActive] = useState("model");
  const [model, setModel] = useState<ModelSettings | null>(null);
  const [chatKey, setChatKey] = useState("");
  const [visionKey, setVisionKey] = useState("");
  const [backups, setBackups] = useState<BackupEntry[]>([]);
  const [pendingRestore, setPendingRestore] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [promptRevision, setPromptRevision] = useState(0);
  const [updateStatus, setUpdateStatus] = useState<UpdateStatusView | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [checkError, setCheckError] = useState("");
  useEffect(() => {
    let alive = true;
    let received = false;
    const off = onUpdateStatus((value) => {
      received = true;
      if (alive) setUpdateStatus(value);
    });
    void getUpdateStatus()
      .then((value) => {
        if (alive && !received) setUpdateStatus(value);
      })
      .catch((error) => {
        if (alive) setCheckError(String(error));
      });
    return () => {
      alive = false;
      void off.then((stop) => stop());
    };
  }, []);
  const checkForUpdate = async () => {
    if (checkingUpdate || updating) return;
    onClearUpdateError?.();
    setCheckingUpdate(true);
    setCheckError("");
    try {
      setUpdateStatus(await checkUpdate());
    } catch (error) {
      setCheckError(String(error));
    } finally {
      setCheckingUpdate(false);
    }
  };
  const updateHint =
    updateError ||
    checkError ||
    (updating
      ? "正在下载并验证更新，完成后会重新启动。"
      : updateStatus?.status === "checking" || checkingUpdate
        ? "正在检查更新…"
        : updateStatus?.status === "error"
          ? updateStatus.error || "检查更新失败，请重试。"
          : updateStatus?.update
            ? `发现新版本 ${updateStatus.update.version}`
            : updateStatus?.status === "upToDate"
              ? "当前已是最新版本。"
              : "启动时会在后台检查更新，也可以手动检查。");
  const load = () =>
    Promise.all([
      getModelSettings().then(setModel),
      listBackups().then(setBackups),
      getPreferences().then(setPreferences),
      getBootstrap().then((b) => {
        setLook(b.config.appearance);
        setGlassSupported(b.glassSupported);
      }),
    ]);
  useEffect(() => {
    void load().catch((error) => setMessage(String(error)));
    const off = onConfigChanged((config) => {
      setLook(config.appearance);
      void getPreferences()
        .then(setPreferences)
        .catch((e) => setMessage(String(e)));
    });
    return () => {
      void off.then((f) => f());
    };
  }, []);
  const act = async (action: () => Promise<void>) => {
    setBusy(true);
    setMessage("");
    try {
      await action();
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  };
  const save = async () => {
    if (!model) return;
    const next = await saveModelSettings({
      chat: model.chat,
      vision: model.vision,
      chatKey,
      visionKey,
    });
    setChatKey("");
    setVisionKey("");
    setModel(next);
    return next;
  };
  const clearKey = async (endpoint: Endpoint) => {
    await clearApiKey(endpoint);
    if (!model) return;
    const chatCleared = sameCredentialScope(model.chat, endpoint);
    const visionCleared = !!model.vision && sameCredentialScope(model.vision, endpoint);
    if (chatCleared) setChatKey("");
    if (visionCleared) setVisionKey("");
    setModel({
      ...model,
      chat: chatCleared ? { ...model.chat, hasKey: false } : model.chat,
      vision: model.vision && visionCleared ? { ...model.vision, hasKey: false } : model.vision,
    });
    setMessage("已清除当前接口地址的 API Key。");
  };
  return (
    <div className="settings-layout">
      <nav aria-label="设置分组">
        <button className="btn ghost" onClick={onBack}>
          ‹ 返回
        </button>
        {[
          ["model", "模型"],
          ["prompts", "提示词"],
          ["time", "时间"],
          ["look", "外观"],
          ["sticky", "便利贴"],
          ["keys", "快捷键"],
          ["startup", "启动"],
          ["report", "报告"],
          ["data", "数据"],
          ["about", "关于"],
        ].map(([id, label]) => (
          <a
            key={id}
            className={active === id ? "on" : ""}
            href={`#${id}`}
            onClick={(e) => {
              e.preventDefault();
              document.getElementById(id)?.scrollIntoView({ behavior: "instant" });
              setActive(id);
            }}
          >
            {label}
          </a>
        ))}
      </nav>
      <div
        className="settings-content"
        onScroll={(e) => {
          const top = e.currentTarget.getBoundingClientRect().top + 40;
          const sections = Array.from(e.currentTarget.querySelectorAll("section"));
          const current = sections.filter((s) => s.getBoundingClientRect().top <= top).pop();
          if (current) setActive(current.id);
        }}
      >
        <div className="settings-inner">
          {message && (
            <p className="settings-message" role="status">
              {message}
            </p>
          )}

          <section id="model">
            <h2>模型</h2>
            <div className="card settings-group">
              {onboarding && (
                <p className="settings-note">先配置对话模型，保存后试着说第一句话。</p>
              )}
              {model && (
                <fieldset disabled={busy}>
                  <EndpointFields
                    value={model.chat}
                    secret={chatKey}
                    onSecret={setChatKey}
                    onClear={() => void act(() => clearKey(model.chat))}
                    onChange={(chat) =>
                      setModel({
                        ...model,
                        chat: {
                          ...chat,
                          hasKey: sameCredentialScope(chat, model.chat) && model.chat.hasKey,
                        },
                      })
                    }
                  />
                  <Row label="识图模型" hint="用于识别截图和照片，可以和对话模型不同">
                    <select
                      className="field wide"
                      value={model.vision ? "separate" : "same"}
                      onChange={(e) => {
                        setVisionKey("");
                        setModel({
                          ...model,
                          vision: e.target.value === "same" ? null : { ...model.chat },
                        });
                      }}
                    >
                      <option value="same">与对话模型相同</option>
                      <option value="separate">单独配置识图模型</option>
                    </select>
                  </Row>
                  {model.vision && (
                    <EndpointFields
                      vision
                      value={model.vision}
                      secret={visionKey}
                      onSecret={setVisionKey}
                      onClear={() => void act(() => clearKey(model.vision!))}
                      onChange={(vision) =>
                        setModel({
                          ...model,
                          vision: {
                            ...vision,
                            hasKey:
                              !!model.vision &&
                              sameCredentialScope(vision, model.vision) &&
                              model.vision.hasKey,
                          },
                        })
                      }
                    />
                  )}
                  <Row label="保存配置">
                    <button
                      className="btn primary"
                      onClick={() =>
                        void act(async () => {
                          const saved = await save();
                          setMessage("模型配置已保存。");
                          if (
                            saved?.chat.hasKey &&
                            saved.chat.model.trim() &&
                            saved.chat.baseUrl.trim()
                          )
                            onModelSaved?.();
                        })
                      }
                    >
                      保存
                    </button>
                  </Row>
                  <Row label="连接测试" hint="保存当前配置后测试">
                    <div className="reply-actions">
                      <button
                        className="btn"
                        onClick={() =>
                          void act(async () => {
                            await save();
                            const result = await testModel("chat");
                            setMessage(`${result.message} · ${result.latencyMs} ms`);
                          })
                        }
                      >
                        测试对话连接
                      </button>
                      <button
                        className="btn"
                        onClick={() =>
                          void act(async () => {
                            await save();
                            const result = await testModel("vision");
                            setMessage(`${result.message} · ${result.latencyMs} ms`);
                          })
                        }
                      >
                        测试识图连接
                      </button>
                    </div>
                  </Row>
                </fieldset>
              )}
              <div className="settings-note">
                数据保存在这台电脑上；使用 AI
                功能时，会把完成本次请求所需的文字或图片发送给你选择的模型服务商。
              </div>
            </div>
          </section>
          <PromptSettings key={promptRevision} report={setMessage} />
          {preferences && appearance && (
            <PreferenceSections
              value={preferences}
              appearance={appearance}
              glassSupported={glassSupported}
              busy={busy}
              save={(p) =>
                void act(async () => {
                  setPreferences(await savePreferences(p));
                })
              }
              look={(a) =>
                void act(async () => {
                  setLook((await setAppearance(a)).appearance);
                })
              }
            />
          )}
          <section id="data">
            <h2>数据</h2>
            <div className="card settings-group">
              <Row label="数据目录" hint={dataDir}>
                <button
                  className="btn"
                  disabled={busy}
                  onClick={() =>
                    void act(async () => {
                      await openFolder("data");
                    })
                  }
                >
                  打开文件夹
                </button>
              </Row>
              <DataExchange report={setMessage} onImported={load} />
              <Row
                label="自动备份"
                hint="每天备份一次，保留最近 30 份；手动和操作前备份保留最近 10 份。包含数据库、报告、配置和提示词，不含 API Key。"
              >
                <button
                  className="btn"
                  disabled={busy}
                  onClick={() =>
                    void act(async () => {
                      await createBackup();
                      setBackups(await listBackups());
                      setMessage("备份已完成。");
                    })
                  }
                >
                  立即备份
                </button>
              </Row>
              {backups.map((backup) => (
                <Row key={backup.name} label={backup.createdAt} hint={backup.name}>
                  <button
                    className="btn"
                    disabled={busy}
                    onClick={() => setPendingRestore(backup.name)}
                  >
                    恢复
                  </button>
                </Row>
              ))}
              {pendingRestore && (
                <div className="settings-note">
                  <p>恢复「{pendingRestore}」将替换当前数据，恢复前会自动备份。</p>
                  <div className="reply-actions">
                    <button
                      className="btn primary"
                      disabled={busy}
                      onClick={() =>
                        void act(async () => {
                          const result = await restoreBackup(pendingRestore);
                          setPendingRestore(null);
                          await load();
                          setPromptRevision((n) => n + 1);
                          setMessage(
                            result.needsApiKey
                              ? "已恢复，请重新填写模型 API Key。"
                              : "数据已恢复。",
                          );
                        })
                      }
                    >
                      确认恢复
                    </button>
                    <button className="btn" disabled={busy} onClick={() => setPendingRestore(null)}>
                      算了
                    </button>
                  </div>
                </div>
              )}
            </div>
          </section>
          <section id="about">
            <h2>关于</h2>
            <div className="card settings-group">
              <Row label="知了" hint={`版本 ${packageInfo.version} · 你只管说，它来整理`}>
                <span className="settings-note">许可证：AGPL-3.0-only</span>
              </Row>
              <Row label="程序更新" hint={updateHint}>
                <div className="reply-actions">
                  <button
                    className="btn"
                    disabled={checkingUpdate || updateStatus?.status === "checking" || updating}
                    onClick={() => void checkForUpdate()}
                  >
                    {checkingUpdate || updateStatus?.status === "checking" ? "检查中…" : "检查更新"}
                  </button>
                  {updateStatus?.update && (
                    <button
                      className="btn primary"
                      disabled={
                        updating ||
                        checkingUpdate ||
                        updateStatus.status === "checking" ||
                        !onInstallUpdate
                      }
                      onClick={onInstallUpdate}
                    >
                      {updating ? "正在更新" : "更新并重启"}
                    </button>
                  )}
                </div>
              </Row>
              {updateStatus?.checkedAt && (
                <p className="settings-note">
                  上次检查：
                  {new Date(updateStatus.checkedAt).toLocaleString("zh-CN", { hour12: false })}
                </p>
              )}
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}
