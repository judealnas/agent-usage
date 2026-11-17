import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

import {
  formatAgo,
  formatCountdown,
  formatLocalTime,
  formatNumber,
  joinInterval,
  splitInterval,
  UNIT_SECONDS,
  type IntervalUnit,
} from "./format";

// ---- Types mirrored from the Rust backend ----

type ProviderId = string;

interface WindowMetric {
  kind: "window";
  label: string;
  duration_minutes: number | null;
  used_percent: number;
  remaining_percent: number;
  resets_at: number | null;
}

interface BalanceMetric {
  kind: "balance";
  label: string;
  used: number | null;
  limit: number | null;
  remaining: number | null;
  currency: string | null;
}

type Metric = WindowMetric | BalanceMetric;

interface Report {
  provider: ProviderId;
  plan: string | null;
  metrics: Metric[];
  limit_reached: boolean;
  details: Record<string, string>;
  fetched_at: number;
  error: string | null;
}

interface ProviderSettings {
  id: ProviderId;
  shown: boolean;
  auto_refresh: boolean;
  interval_seconds: number;
}

interface Settings {
  providers: ProviderSettings[];
  codex: { model: string; effort: string; auth_file: string | null };
  claude: { credentials_file: string | null };
  thresholds: { warn: number; critical: number };
  warn_fast_codex_polling: boolean;
  pinned: boolean;
}

interface Meta {
  providers: { id: ProviderId; name: string }[];
  codex_efforts: string[];
}

/** What a card knows: the last good report, and the latest failure if any. */
interface CardState {
  good: Report | null;
  error: string | null;
}

// ---- State ----

const appWindow = getCurrentWindow();
const appEl = document.getElementById("app")!;
const contentEl = document.getElementById("content")!;
const addMenuEl = document.getElementById("add-menu")!;
const titlebarButtonsEl = document.getElementById("titlebar-buttons")!;

let settings: Settings;
let meta: Meta;
let view: "cards" | "settings" = "cards";
const cardStates = new Map<ProviderId, CardState>();
const cardEls = new Map<ProviderId, HTMLElement>();
let tickTimer: number | undefined;

// ---- Small DOM helpers ----

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

const ICONS: Record<string, string> = {
  pin: "M6 2h4l-.5 4.5L12 9H4l2.5-2.5zM8 9v5",
  refresh: "M13 8a5 5 0 1 1-1.6-3.7M13 2.5v3h-3",
  plus: "M8 3v10M3 8h10",
  settings: "M2 5h7M12 5h2M2 11h2M7 11h7M9 3.5v3M4 9.5v3",
  minimize: "M3.5 11.5h9",
  close: "M4 4l8 8M12 4l-8 8",
};

function iconButton(icon: string, label: string, onClick: () => void): HTMLButtonElement {
  const button = el("button", "icon-button");
  button.type = "button";
  button.title = label;
  button.setAttribute("aria-label", label);
  button.innerHTML = `<svg viewBox="0 0 16 16"><path d="${ICONS[icon]}"/></svg>`;
  button.addEventListener("click", onClick);
  return button;
}

function providerName(id: ProviderId): string {
  return meta.providers.find((p) => p.id === id)?.name ?? id;
}

function shownProviders(): ProviderSettings[] {
  return settings.providers.filter((p) => p.shown);
}

function nowSeconds(): number {
  return Date.now() / 1000;
}

// ---- Title bar ----

let pinButton: HTMLButtonElement;
let addButton: HTMLButtonElement;

function buildTitlebar(): void {
  pinButton = iconButton("pin", "Keep on top", async () => {
    settings = await invoke<Settings>("set_pinned", { pinned: !settings.pinned });
    updateTitlebar();
  });
  const refreshAll = iconButton("refresh", "Refresh all", () => void invoke("refresh_all"));
  addButton = iconButton("plus", "Add provider", () => {
    addMenuEl.hidden = !addMenuEl.hidden;
  });
  const gear = iconButton("settings", "Settings", () => {
    view = view === "settings" ? "cards" : "settings";
    render();
  });
  const minimize = iconButton("minimize", "Minimize", () => void appWindow.minimize());
  const close = iconButton("close", "Close", () => void appWindow.close());
  close.classList.add("close");
  titlebarButtonsEl.replaceChildren(pinButton, refreshAll, addButton, gear, minimize, close);
}

function updateTitlebar(): void {
  pinButton.classList.toggle("active", settings.pinned);
  pinButton.title = settings.pinned ? "On top (click to unpin)" : "Keep on top";

  const hidden = settings.providers.filter((p) => !p.shown);
  addButton.disabled = hidden.length === 0 || view !== "cards";
  if (addButton.disabled) addMenuEl.hidden = true;

  addMenuEl.replaceChildren(
    ...hidden.map((p) => {
      const item = el("button", undefined, `Add ${providerName(p.id)}`);
      item.type = "button";
      item.addEventListener("click", async () => {
        addMenuEl.hidden = true;
        settings = await invoke<Settings>("set_shown", { provider: p.id, shown: true });
        render();
      });
      return item;
    }),
  );
}

// ---- Cards ----

function buildCard(provider: ProviderSettings): HTMLElement {
  const id = provider.id;
  const card = el("section", "card");
  card.dataset.id = id;

  const header = el("div", "card-header");
  header.title = "Drag to reorder";
  const title = el("div", "card-title");
  const hide = iconButton("close", `Hide ${providerName(id)}`, async () => {
    settings = await invoke<Settings>("set_shown", { provider: id, shown: false });
    render();
  });
  header.append(title, hide);
  enableDragging(card, header);

  const body = el("div", "card-body");
  const notice = el("div", "notice");
  const status = el("div", "status");

  const footer = el("div", "card-footer");
  const refresh = iconButton("refresh", "Refresh now", () => void invoke("refresh", { provider: id }));

  const switchLabel = el("label", "switch");
  const toggle = el("input");
  toggle.type = "checkbox";
  toggle.checked = provider.auto_refresh;
  switchLabel.append(toggle, el("span", undefined, "Auto Refresh"));

  const interval = el("span", "interval");
  const split = splitInterval(provider.interval_seconds);
  const valueInput = el("input");
  valueInput.type = "number";
  valueInput.min = "0";
  valueInput.step = "any";
  valueInput.value = String(split.value);
  valueInput.title = "Refresh interval (at least 1 second)";
  const unitSelect = el("select");
  for (const unit of Object.keys(UNIT_SECONDS)) {
    const option = el("option", undefined, unit);
    option.value = unit;
    unitSelect.append(option);
  }
  unitSelect.value = split.unit;
  interval.append(valueInput, unitSelect);
  interval.hidden = !provider.auto_refresh;

  const current = () => settings.providers.find((p) => p.id === id)!;

  const apply = async (autoRefresh: boolean, seconds: number) => {
    try {
      settings = await invoke<Settings>("set_auto_refresh", {
        provider: id,
        autoRefresh,
        intervalSeconds: seconds,
      });
    } catch (error) {
      console.error(error);
    }
    interval.hidden = !current().auto_refresh;
    updateCard(id);
    resetTick();
  };

  toggle.addEventListener("change", () => void apply(toggle.checked, current().interval_seconds));

  const applyInterval = () => {
    const seconds = joinInterval(Number(valueInput.value), unitSelect.value as IntervalUnit);
    valueInput.classList.toggle("invalid", seconds === null);
    if (seconds === null) {
      valueInput.title = "The interval must be at least 1 second";
      return;
    }
    valueInput.title = "Refresh interval (at least 1 second)";
    void apply(true, seconds);
  };
  // `change` fires on Enter and on blur.
  valueInput.addEventListener("change", applyInterval);
  unitSelect.addEventListener("change", applyInterval);

  footer.append(refresh, switchLabel, interval);
  card.append(header, body, notice, status, footer);
  return card;
}

function barLevel(usedPercent: number): string {
  if (usedPercent >= settings.thresholds.critical) return "critical";
  if (usedPercent >= settings.thresholds.warn) return "warn";
  return "";
}

function renderMetric(metric: Metric, fetchedAt: number): HTMLElement {
  const row = el("div", "metric");
  const top = el("div", "metric-top");
  row.append(top);
  top.append(el("span", undefined, metric.label));

  if (metric.kind === "window") {
    const now = nowSeconds();
    // The window rolled over after this reading was taken.
    const hasReset =
      metric.resets_at !== null && metric.resets_at <= now && metric.resets_at > fetchedAt;
    const used = hasReset ? 0 : metric.used_percent;

    top.append(el("span", "value", `${formatNumber(used)}%`));
    const bar = el("div", "bar");
    const fill = el("div", `fill ${barLevel(used)}`);
    fill.style.width = `${Math.min(100, Math.max(0, used))}%`;
    bar.append(fill);
    row.append(bar);

    if (hasReset) {
      row.append(el("div", "metric-sub", "reset, awaiting refresh"));
    } else if (metric.resets_at !== null) {
      const sub = el("div", "metric-sub", `resets in ${formatCountdown(metric.resets_at - now)}`);
      sub.title = formatLocalTime(metric.resets_at);
      row.append(sub);
    }
  } else {
    const unit = metric.currency ? ` ${metric.currency}` : "";
    const parts: string[] = [];
    if (metric.used !== null && metric.limit !== null) {
      parts.push(`${formatNumber(metric.used)} of ${formatNumber(metric.limit)}${unit} used`);
    } else if (metric.used !== null) {
      parts.push(`${formatNumber(metric.used)}${unit} used`);
    }
    if (metric.remaining !== null) {
      parts.push(`${formatNumber(metric.remaining)}${unit} left`);
    }
    top.append(el("span", "value", parts.join(", ")));
    if (metric.used !== null && metric.limit !== null && metric.limit > 0) {
      const percent = (metric.used / metric.limit) * 100;
      const bar = el("div", "bar");
      const fill = el("div", `fill ${barLevel(percent)}`);
      fill.style.width = `${Math.min(100, Math.max(0, percent))}%`;
      bar.append(fill);
      row.append(bar);
    }
  }
  return row;
}

/** Redraw the parts of a card that depend on reports and the clock. */
function updateCard(id: ProviderId): void {
  const card = cardEls.get(id);
  const provider = settings.providers.find((p) => p.id === id);
  if (!card || !provider) return;
  const state = cardStates.get(id) ?? { good: null, error: null };
  const { good, error } = state;

  const title = card.querySelector(".card-title")!;
  title.replaceChildren(el("span", undefined, providerName(id)));
  if (good?.plan) title.append(el("span", "plan", ` · ${good.plan}`));

  const body = card.querySelector(".card-body")!;
  if (good) {
    body.replaceChildren(...good.metrics.map((metric) => renderMetric(metric, good.fetched_at)));
    if (good.limit_reached) body.append(el("div", "message", "Limit reached."));
  } else if (error) {
    body.replaceChildren(el("div", "message error", error));
  } else {
    body.replaceChildren(el("div", "message", "Loading…"));
  }

  const stale = good !== null && error !== null;
  card.classList.toggle("stale", stale);

  const status = card.querySelector<HTMLElement>(".status")!;
  status.hidden = good === null;
  if (good) {
    const updated = `updated ${formatAgo(nowSeconds() - good.fetched_at)}`;
    if (stale) {
      const mark = el("span", "stale-mark", "stale");
      mark.title = error!;
      status.replaceChildren(mark, ` · ${updated}`);
    } else {
      status.replaceChildren(updated);
    }
  }

  const notice = card.querySelector<HTMLElement>(".notice")!;
  const fastCodex =
    id === "codex" &&
    settings.warn_fast_codex_polling &&
    provider.auto_refresh &&
    provider.interval_seconds < 60;
  notice.hidden = !fastCodex;
  notice.textContent = fastCodex
    ? "Every Codex refresh is a real request that uses a little of your quota."
    : "";
}

function renderCards(): void {
  cardEls.clear();
  const shown = shownProviders();
  if (shown.length === 0) {
    contentEl.replaceChildren(
      el("div", "empty", "No providers shown. Use the + button above to add one."),
    );
    return;
  }
  const list = el("div", "cards");
  for (const provider of shown) {
    const card = buildCard(provider);
    cardEls.set(provider.id, card);
    list.append(card);
  }
  contentEl.replaceChildren(list);
  for (const provider of shown) updateCard(provider.id);
}

// ---- Drag to reorder ----

function enableDragging(card: HTMLElement, header: HTMLElement): void {
  // Moving the card in the DOM drops any pointer capture, so the move and
  // release are tracked on the window for the length of the drag.
  const onMove = (event: PointerEvent) => {
    const over = document
      .elementFromPoint(event.clientX, event.clientY)
      ?.closest<HTMLElement>(".card");
    const list = card.parentElement;
    if (!over || over === card || !list || over.parentElement !== list) return;
    const cards = Array.from(list.children);
    // Moving the card into the other's place shifts the rest along.
    if (cards.indexOf(card) < cards.indexOf(over)) over.after(card);
    else over.before(card);
  };

  const finish = async () => {
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", finish);
    window.removeEventListener("pointercancel", finish);
    card.classList.remove("dragging");

    const list = card.parentElement;
    if (!list) return;
    const order = Array.from(list.children).map((child) => (child as HTMLElement).dataset.id!);
    const before = shownProviders().map((p) => p.id);
    if (order.join() === before.join()) return;
    settings = await invoke<Settings>("set_order", { order });
  };

  header.addEventListener("pointerdown", (event) => {
    if (event.button !== 0 || (event.target as HTMLElement).closest("button")) return;
    event.preventDefault();
    card.classList.add("dragging");
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
  });
}

// ---- Settings view ----

function renderSettings(): void {
  const form = el("form", "settings");

  const textField = (label: string, value: string, placeholder = "") => {
    const field = el("label", "field");
    const input = el("input");
    input.type = "text";
    input.value = value;
    input.placeholder = placeholder;
    input.spellcheck = false;
    field.append(el("span", undefined, label), input);
    return { field, input };
  };
  const numberField = (label: string, value: number) => {
    const field = el("label", "field");
    const input = el("input");
    input.type = "number";
    input.min = "0";
    input.max = "100";
    input.step = "any";
    input.value = String(value);
    field.append(el("span", undefined, label), input);
    return { field, input };
  };

  const model = textField("Model", settings.codex.model);
  const effortField = el("label", "field");
  const effort = el("select");
  for (const value of meta.codex_efforts) {
    const option = el("option", undefined, value);
    option.value = value;
    effort.append(option);
  }
  effort.value = settings.codex.effort;
  effortField.append(el("span", undefined, "Effort"), effort);
  const codexRow = el("div", "row");
  codexRow.append(model.field, effortField);

  const authFile = textField(
    "Codex auth file",
    settings.codex.auth_file ?? "",
    "Default: ~/.codex/auth.json",
  );
  const credentialsFile = textField(
    "Claude credentials file",
    settings.claude.credentials_file ?? "",
    "Default: ~/.claude/.credentials.json",
  );

  const warn = numberField("Amber from (%)", settings.thresholds.warn);
  const critical = numberField("Red from (%)", settings.thresholds.critical);
  const thresholdRow = el("div", "row");
  thresholdRow.append(warn.field, critical.field);

  const fastLabel = el("label", "switch");
  const fast = el("input");
  fast.type = "checkbox";
  fast.checked = settings.warn_fast_codex_polling;
  fastLabel.append(fast, el("span", undefined, "Warn when Codex refreshes faster than once a minute"));

  const errorEl = el("div", "form-error");
  errorEl.hidden = true;

  const actions = el("div", "actions");
  const cancel = el("button", undefined, "Cancel");
  cancel.type = "button";
  cancel.addEventListener("click", () => {
    view = "cards";
    render();
  });
  const save = el("button", "primary", "Save");
  save.type = "submit";
  actions.append(cancel, save);

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const preferences = {
      codex: {
        model: model.input.value,
        effort: effort.value,
        auth_file: authFile.input.value.trim() || null,
      },
      claude: { credentials_file: credentialsFile.input.value.trim() || null },
      thresholds: { warn: Number(warn.input.value), critical: Number(critical.input.value) },
      warn_fast_codex_polling: fast.checked,
    };
    try {
      settings = await invoke<Settings>("save_preferences", { preferences });
      view = "cards";
      render();
    } catch (error) {
      errorEl.textContent = String(error);
      errorEl.hidden = false;
    }
  });

  form.append(
    el("h2", undefined, "Codex"),
    codexRow,
    authFile.field,
    el("h2", undefined, "Claude Code"),
    credentialsFile.field,
    el("h2", undefined, "Bar colors"),
    thresholdRow,
    fastLabel,
    errorEl,
    actions,
  );
  contentEl.replaceChildren(form);
}

// ---- Rendering, ticking, and window height ----

function render(): void {
  updateTitlebar();
  if (view === "settings") renderSettings();
  else renderCards();
  resetTick();
}

/** Keep countdowns and "updated … ago" current without re-fetching. */
function resetTick(): void {
  window.clearInterval(tickTimer);
  const fast = shownProviders().some((p) => p.auto_refresh && p.interval_seconds < 60);
  tickTimer = window.setInterval(
    () => {
      if (view !== "cards") return;
      for (const id of cardEls.keys()) updateCard(id);
    },
    fast ? 1000 : 15000,
  );
}

/** Make the window exactly as tall as its content. */
async function fitHeight(): Promise<void> {
  const height = Math.ceil(appEl.getBoundingClientRect().height);
  if (height < 1 || Math.abs(window.innerHeight - height) < 1) return;
  try {
    await appWindow.setSize(new LogicalSize(window.innerWidth, height));
  } catch (error) {
    console.error(error);
  }
}

async function init(): Promise<void> {
  document.addEventListener("contextmenu", (event) => event.preventDefault());

  [meta, settings] = await Promise.all([
    invoke<Meta>("get_meta"),
    invoke<Settings>("get_settings"),
  ]);
  buildTitlebar();
  render();

  new ResizeObserver(() => void fitHeight()).observe(appEl);
  window.addEventListener("resize", () => void fitHeight());

  await listen<Report>("usage-report", (event) => {
    const report = event.payload;
    const state = cardStates.get(report.provider) ?? { good: null, error: null };
    if (report.error) {
      state.error = report.error;
    } else {
      state.good = report;
      state.error = null;
    }
    cardStates.set(report.provider, state);
    updateCard(report.provider);
  });

  // The backend waits for this before fetching, so no report is missed.
  await invoke("start");
}

void init();
