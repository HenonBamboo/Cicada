import type { Appearance, Scale } from "../lib/types";
import type { Preferences } from "../lib/settings";
import { THEMES } from "../theme/resolve";
import { Row, Seg, Shortcut, Switch } from "./SettingsControls";
const previews = [
  ["linear-gradient(90deg,#F7F8F7 50%,#16191A 50%)", "#C9D2CF", "#9993", "#16796F"],
  ["#F7F8F7", "#DDE3E0", "#fff", "#16796F"],
  ["#16191A", "#2F3637", "#1E2223", "#3FB3A2"],
  ["#F4EFE6", "#E3D8C5", "#FBF8F2", "#9A6B2F"],
  [
    "radial-gradient(at 20% 25%,#BFE3D8,transparent 60%),radial-gradient(at 85% 80%,#F6D9B8,transparent 60%),#E9EEEC",
    "#ffffffb3",
    "#ffffff99",
    "#16796F",
  ],
  ["#17211E", "#2C3A35", "#1E2A26", "#7CCBA2"],
];
const scales: [Scale, string][] = [
  ["small", "小"],
  ["medium", "中"],
  ["large", "大"],
];
export function PreferenceSections({
  value,
  appearance,
  glassSupported,
  save,
  look,
  busy,
}: {
  value: Preferences;
  appearance: Appearance;
  glassSupported: boolean;
  save: (p: Preferences) => void;
  look: (p: Appearance) => void;
  busy: boolean;
}) {
  const time = (p: Partial<Preferences["time"]>) =>
    save({ ...value, time: { ...value.time, ...p } });
  const desktop = (p: Partial<Preferences["desktop"]>) =>
    save({ ...value, desktop: { ...value.desktop, ...p } });
  const automation = (p: Partial<Preferences["automation"]>) =>
    save({ ...value, automation: { ...value.automation, ...p } });
  const sticky = (p: Partial<Preferences["sticky"]>) =>
    save({ ...value, sticky: { ...value.sticky, ...p } });
  return (
    <fieldset className="settings-fields" disabled={busy}>
      <section id="time">
        <h2>时间</h2>
        <div className="card settings-group">
          <Row label="日切时间" hint="过了这个时间才算新的一天，夜猫子可以设晚一点">
            <input
              aria-label="日切时间"
              className="field"
              type="time"
              defaultValue={value.time.dayCutoff}
              key={value.time.dayCutoff}
              onBlur={(e) =>
                e.target.value !== value.time.dayCutoff && time({ dayCutoff: e.target.value })
              }
            />
          </Row>
          <Row label="每周从哪天开始" hint="影响周报的统计范围">
            <Seg
              value={value.time.weekStart}
              choices={[
                ["monday", "周一"],
                ["sunday", "周日"],
              ]}
              onChange={(weekStart) => time({ weekStart })}
            />
          </Row>
          <Row label="每日回顾" hint="每晚提醒一次：今天完成了几件，明天要做什么">
            <input
              aria-label="每日回顾时间"
              className="field"
              type="time"
              defaultValue={value.automation.reviewTime}
              key={value.automation.reviewTime}
              onBlur={(e) =>
                e.target.value !== value.automation.reviewTime &&
                automation({ reviewTime: e.target.value })
              }
            />
            <Switch
              label="每日回顾"
              value={value.automation.reviewEnabled}
              onChange={(reviewEnabled) => automation({ reviewEnabled })}
            />
          </Row>
        </div>
      </section>
      <section id="look">
        <h2>外观</h2>
        <div className="card settings-group">
          <div className="settings-row col">
            <div className="lab">
              <b>主题</b>
            </div>
            <div className="themes">
              {THEMES.map((t, i) => (
                <button
                  key={t.id}
                  className={`theme-card ${appearance.theme === t.id ? "on" : ""}`}
                  onClick={() => look({ ...appearance, theme: t.id })}
                >
                  <div className="pv" style={{ background: previews[i][0] }}>
                    <i style={{ width: "60%", background: previews[i][1] }} />
                    <i style={{ width: "38%", background: previews[i][1] }} />
                    <div className="pc" style={{ background: previews[i][2] }}>
                      <span className="dot" style={{ background: previews[i][3] }} />
                      <i style={{ flex: 1, background: previews[i][1] }} />
                    </div>
                  </div>
                  <div className="nm">
                    {t.name}
                    <span>{appearance.theme === t.id ? "✓" : ""}</span>
                  </div>
                </button>
              ))}
            </div>
            {appearance.theme === "glass" && !glassSupported && (
              <small>此系统不支持毛玻璃，已使用浅色主题。</small>
            )}
          </div>
          <Row label="强调色" hint="按钮、勾选框、热力图都会跟着变">
            <div className="accents">
              <button
                className={`auto ${!appearance.accent ? "on" : ""}`}
                onClick={() => look({ ...appearance, accent: null })}
              >
                跟随主题
              </button>
              {["#16796F", "#B7791F", "#4F5BD5", "#C2457A", "#4B5563"].map((c) => (
                <button
                  key={c}
                  aria-label={c}
                  className={appearance.accent === c ? "on" : ""}
                  style={{ background: c }}
                  onClick={() => look({ ...appearance, accent: c })}
                />
              ))}
              <input
                type="color"
                aria-label="自定义强调色"
                defaultValue={appearance.accent || "#16796F"}
                key={appearance.accent}
                onBlur={(e) =>
                  e.target.value !== appearance.accent &&
                  look({ ...appearance, accent: e.target.value })
                }
              />
            </div>
          </Row>
          <Row label="字号">
            <Seg
              value={appearance.fontSize}
              choices={scales}
              onChange={(fontSize) => look({ ...appearance, fontSize })}
            />
          </Row>
          <Row label="圆角">
            <Seg
              value={appearance.radius}
              choices={scales}
              onChange={(radius) => look({ ...appearance, radius })}
            />
          </Row>
          <Row label="动效" hint="关闭后所有过渡都会变成瞬时的">
            <Switch
              label="动效"
              value={appearance.motion}
              onChange={(motion) => look({ ...appearance, motion })}
            />
          </Row>
        </div>
      </section>
      <section id="sticky">
        <h2>便利贴</h2>
        <div className="card settings-group">
          <Row label="默认颜色" hint="用于新建便利贴">
            <div className="accents">
              {[
                ["amber", "#FFE7A6", "黄"],
                ["pink", "#F9C9D3", "粉"],
                ["blue", "#BFD6F5", "蓝"],
                ["green", "#BDE3D1", "绿"],
              ].map(([c, b, l]) => (
                <button
                  key={c}
                  aria-label={l}
                  className={value.sticky.color === c ? "on" : ""}
                  style={{ background: b }}
                  onClick={() => sticky({ color: c })}
                />
              ))}
              <button
                className={`auto ${value.sticky.color === "theme" ? "on" : ""}`}
                onClick={() => sticky({ color: "theme" })}
              >
                跟随主题
              </button>
            </div>
          </Row>
          <Row label="透明度">
            <input
              aria-label="便利贴透明度"
              className="range"
              type="range"
              min="55"
              max="100"
              defaultValue={Math.round(value.sticky.opacity * 100)}
              key={value.sticky.opacity}
              onPointerUp={(e) => sticky({ opacity: Number(e.currentTarget.value) / 100 })}
              onKeyUp={(e) => sticky({ opacity: Number(e.currentTarget.value) / 100 })}
            />
          </Row>
          <Row label="贴边自动隐藏" hint="拖到屏幕边缘后收起，鼠标靠近时再滑出">
            <Switch
              label="贴边自动隐藏"
              value={value.sticky.autoHide}
              onChange={(autoHide) => sticky({ autoHide })}
            />
          </Row>
        </div>
      </section>
      <section id="keys">
        <h2>快捷键</h2>
        <div className="card settings-group">
          <Row label="快速记录" hint="在任何软件里都能唤起输入条">
            <Shortcut
              value={value.desktop.quickShortcut}
              onChange={(quickShortcut) => desktop({ quickShortcut })}
            />
          </Row>
          <Row label="显示 / 隐藏主窗口">
            <Shortcut
              value={value.desktop.mainShortcut}
              onChange={(mainShortcut) => desktop({ mainShortcut })}
            />
          </Row>
          <Row label="显示 / 隐藏便利贴">
            <Shortcut
              value={value.desktop.stickyShortcut}
              onChange={(stickyShortcut) => desktop({ stickyShortcut })}
            />
          </Row>
        </div>
      </section>
      <section id="startup">
        <h2>启动</h2>
        <div className="card settings-group">
          <Row label="启动界面" hint="下次启动时只显示所选界面">
            <Seg
              value={value.desktop.startupView}
              choices={[
                ["main", "主界面"],
                ["sticky", "便利贴"],
              ]}
              onChange={(startupView) => desktop({ startupView })}
            />
          </Row>
          <Row label="开机自动启动">
            <Switch
              label="开机自动启动"
              value={value.desktop.autostart}
              onChange={(autostart) => desktop({ autostart })}
            />
          </Row>
          <Row label="关闭主窗口时">
            <Seg
              value={value.desktop.closeToTray ? "tray" : "exit"}
              choices={[
                ["tray", "缩到托盘"],
                ["exit", "退出知了"],
              ]}
              onChange={(v) => desktop({ closeToTray: v === "tray" })}
            />
          </Row>
        </div>
      </section>
      <section id="report">
        <h2>报告</h2>
        <div className="card settings-group">
          {(
            [
              ["weeklyReport", "周报"],
              ["monthlyReport", "月报"],
              ["yearlyReport", "年报"],
            ] as const
          ).map(([k, l]) => (
            <Row
              key={k}
              label={`自动生成${l}`}
              hint={k === "weeklyReport" ? "每周第一天日切后生成上一周的报告" : undefined}
            >
              <Switch
                label={`自动生成${l}`}
                value={value.automation[k]}
                onChange={(v) => automation({ [k]: v })}
              />
            </Row>
          ))}
        </div>
      </section>
    </fieldset>
  );
}
