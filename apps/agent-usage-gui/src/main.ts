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

import { usageHue } from "./colors";

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
  groups?: SourceReport["groups"];
  provider: ProviderId;
  plan: string | null;
  metrics: Metric[];
  limit_reached: boolean;
  details: Record<string, string>;
  fetched_at: number;
  error: string | null;
}

type Configuration =
  | { kind: "codex_inference"; options: { model: string; effort: string; auth_file: string | null } }
  | { kind: "codex_account"; options: { executable: string; codex_home: string | null; timeout_seconds: number } }
  | { kind: "claude"; options: { credentials_file: string | null } };
interface SourceReport {
  source: string; fetched_at: number; error: string | null;
  groups: { key: string; limit_id: string | null; limit_name: string | null;
    plan: string | null; metrics: Metric[]; error: string | null;
    ordinary_usage_allowed: boolean | null; spend_control_reached: boolean | null;
    rate_limit_reached_type: string | null }[];
}
interface EntryReport { entry_id: string; generation: number; report: Report | null; source_report: SourceReport | null }
interface ProviderSettings {
  tray_enabled: boolean;
  tray_group: string | null;
  name: string | null;
  config: Configuration;
  generation: number;
  id: ProviderId;
  shown: boolean;
  auto_refresh: boolean;
  interval_seconds: number;
}

interface Settings {
  close_to_tray: boolean;
  providers: ProviderSettings[];
  thresholds: { critical: number };
  warn_fast_polling: boolean;
  pinned: boolean;
}

interface Meta {
  author: string;
  version: string;
  repository: string;
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

function configLabel(config: Configuration): string {
  return config.kind === "codex_inference" ? "Codex — Inference" : config.kind === "codex_account" ? "Codex — App Server" : "Claude";
}
function providerName(id: ProviderId): string {
  const entry = settings.providers.find(p => p.id === id);
  return entry ? entry.name ?? configLabel(entry.config) : id;
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
  addButton = iconButton("plus", "Show hidden provider", () => {
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

/** Shown on a card whose provider is set to refresh faster than once a minute. */
const FAST_REFRESH_NOTICES: Record<ProviderId, string> = {
  codex: "Every Codex refresh is a real request that uses a little of your quota.",
  claude: "Refreshing Claude this often can get usage requests rate-limited for a while.",
};
function buildCard(provider: ProviderSettings): HTMLElement {
  const id = provider.id;
  const card = el("section", "card");
  card.dataset.id = id;

  const header = el("div", "card-header");
  header.title = "Drag to reorder";
  const title = el("div", "card-title");
  const brand = provider.config.kind === "claude" ? "claude" : "openai";
  const logo = el("span", `provider-logo provider-logo-${brand}`);
  logo.setAttribute("role", "img");
  logo.setAttribute("aria-label", brand === "claude" ? "Claude" : "OpenAI");
  const hide = iconButton("close", `Hide ${providerName(id)}`, async () => {
    settings = await invoke<Settings>("set_shown", { provider: id, shown: false });
    render();
  });
  header.append(logo, title, hide);
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
  valueInput.min = "1";
  valueInput.step = "1";
  // Whole numbers only: block the keys that make decimals, signs, and exponents.
  valueInput.addEventListener("keydown", (event) => {
    if ([".", ",", "-", "+", "e", "E"].includes(event.key)) event.preventDefault();
  });
  valueInput.value = String(split.value);
  valueInput.title = "Refresh interval (a whole number, 1 or more)";
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
      valueInput.title = "Enter a whole number, 1 or more";
      return;
    }
    valueInput.title = "Refresh interval (a whole number, 1 or more)";
    void apply(true, seconds);
  };
  // `change` fires on Enter and on blur.
  valueInput.addEventListener("change", applyInterval);
  unitSelect.addEventListener("change", applyInterval);

  footer.append(refresh, switchLabel, interval);
  card.append(header, body, notice, status, footer);
  return card;
}

function createFill(usedPercent: number): HTMLElement {
  const fill = el("div", "fill");
  fill.style.width = `${Math.min(100, Math.max(0, usedPercent))}%`;
  fill.style.setProperty("--usage-hue", String(usageHue(usedPercent, settings.thresholds.critical)));
  return fill;
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
    const used = metric.used_percent;

    top.append(el("span", "value", `${formatNumber(used)}%`));
    const bar = el("div", "bar");
    const fill = createFill(used);
    bar.append(fill);
    row.append(bar);

    if (hasReset) {
      row.append(el("div", "metric-sub", "reset time passed; awaiting refresh"));
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
      const fill = createFill(percent);
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
  if (provider.name) title.append(el("span", "plan", ` · ${configLabel(provider.config)}`));
  if (good?.plan) title.append(el("span", "plan", ` · ${good.plan}`));

  const body = card.querySelector(".card-body")!;
  if (good) {
    if (good.groups) {
      body.replaceChildren();
      for (const group of good.groups) {
        const section = el("section", "quota-group");
        section.append(el("h3", undefined, group.limit_name ?? group.limit_id ?? group.key));
        if (group.plan) section.append(el("div", "metric-sub", group.plan));
        section.append(...group.metrics.map(metric => renderMetric(metric, good.fetched_at)));
        if (group.ordinary_usage_allowed === false) section.append(el("div", "message", "Ordinary usage is not allowed."));
        if (group.spend_control_reached === true) section.append(el("div", "message", "Spend control reached."));
        if (group.rate_limit_reached_type) section.append(el("div", "message", `Limit reached: ${group.rate_limit_reached_type}`));
        if (group.error) section.append(el("div", "message error", group.error));
        body.append(section);
      }
      if (!good.groups.length) body.append(el("div", "message", "No quota groups returned."));
    } else {
      body.replaceChildren(...good.metrics.map((metric) => renderMetric(metric, good.fetched_at)));
      if (good.limit_reached) body.append(el("div", "message", "Limit reached."));
    }
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
  const fast =
    settings.warn_fast_polling && provider.auto_refresh && provider.interval_seconds < 60;
  const noticeType = provider.config.kind === "codex_inference" ? "codex" : provider.config.kind === "claude" ? "claude" : "account";
  const text = fast ? FAST_REFRESH_NOTICES[noticeType] : undefined;
  notice.hidden = text === undefined;
  notice.textContent = text ?? "";
}

function renderCards(): void {
  cardEls.clear();
  const shown = shownProviders();
  if (shown.length === 0) {
    contentEl.replaceChildren(
      el("div", "empty", "No providers shown. Add providers in Settings, or show a hidden card using + above."),
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

interface CliStatus {
  supported: boolean;
  available: boolean;
  enabled: boolean;
  managed: boolean;
  conflicts: string[];
}

function cliSettings(): HTMLElement {
  const section = el("div", "cli-settings");
  const label = el("label", "switch");
  const toggle = el("input");
  toggle.type = "checkbox";
  toggle.disabled = true;
  toggle.setAttribute("aria-label", "Enable command line interface");
  label.append(toggle, el("span", undefined, "Enable agent-usage-cli command"));
  const message = el("p", "setting-help", "Checking CLI installation…");
  message.setAttribute("role", "status");
  const paths = el("ul", "cli-conflicts");
  const error = el("p", "form-error");
  error.setAttribute("role", "alert");
  const check = el("button", undefined, "Check again");
  check.type = "button";
  let status: CliStatus | undefined;
  const display = (next: CliStatus) => {
    status = next;
    toggle.checked = next.enabled;
    // Always allow removal of our own registration, even if a conflict appeared later.
    toggle.disabled = !next.supported || (next.enabled ? !next.managed : !next.available || next.conflicts.length > 0);
    paths.replaceChildren(...next.conflicts.map(path => el("li", undefined, path)));
    message.textContent = !next.supported ? "CLI integration is currently available on Windows only."
      : next.conflicts.length ? "An independently installed CLI was found. Remove it from PATH yourself, then check again to enable the bundled CLI. Your existing installation will not be changed."
      : !next.available ? "The bundled CLI is missing. Reinstall the GUI to restore it."
      : next.enabled && !next.managed ? "The bundled CLI is already on PATH through an entry managed outside this app."
      : next.enabled ? "Enabled. Open a new terminal and run agent-usage-cli. It also works while the GUI is closed."
      : "Make the bundled CLI available in new terminals. Changes apply immediately.";
  };
  const update = async (enabled?: boolean) => {
    toggle.disabled = true;
    check.disabled = true;
    error.textContent = "";
    try {
      display(enabled === undefined
        ? await invoke<CliStatus>("get_cli_status")
        : await invoke<CliStatus>("set_cli_enabled", { enabled }));
    } catch (reason) {
      error.textContent = String(reason);
      // Refresh after failed mutation: the conflict may have appeared since opening settings.
      try { display(await invoke<CliStatus>("get_cli_status")); }
      catch { if (status) toggle.checked = status.enabled; toggle.disabled = true; }
    } finally {
      check.disabled = false;
    }
  };
  toggle.addEventListener("change", () => void update(toggle.checked));
  check.addEventListener("click", () => void update());
  section.append(el("h3", undefined, "Command line interface"), label, message, paths, error, check);
  void update();
  return section;
}

let preferencesSave: Promise<void> = Promise.resolve();

function populateTrayGroups(select: HTMLSelectElement, entry: ProviderSettings): void {
  const groups = cardStates.get(entry.id)?.good?.groups ?? [];
  const choices: [string, string][] = [["", "Use the only available group"]];
  for (const group of groups) choices.push([group.key, group.limit_name ?? group.limit_id ?? group.key]);
  if (entry.tray_group && !groups.some(g => g.key === entry.tray_group)) choices.push([entry.tray_group, `${entry.tray_group} (unavailable)`]);
  select.replaceChildren(...choices.map(([value, name]) => {
    const option = el("option", undefined, name); option.value = value; return option;
  }));
  select.value = entry.tray_group ?? "";
  select.parentElement!.hidden = groups.length <= 1 && !entry.tray_group;
}

function renderSettings(): void {
  const form = el("form", "settings settings-overview");

  const numberField = (label: string, value: number) => {
    const field = el("label", "field");
    const input = el("input");
    input.type = "number";
    input.min = "1";
    input.max = "100";
    input.step = "any";
    input.required = true;
    input.value = String(value);
    field.append(el("span", undefined, label), input);
    return { field, input };
  };

  const critical = numberField("Red at (% used)", settings.thresholds.critical);
  const colorHelp = el("p", "setting-help", "Bars shift smoothly from green at 0% through yellow to red at this percentage, and stay red above it.");
  const preview = el("div", "color-preview");
  preview.setAttribute("aria-hidden", "true");
  const previewLabels = el("div", "color-preview-labels");
  const redLabel = el("span");
  previewLabels.append(el("span", undefined, "0% used"), redLabel);
  const updatePreview = () => {
    const threshold = Number(critical.input.value);
    redLabel.textContent = critical.input.validity.valid ? `${formatNumber(threshold)}% used` : "Red threshold";
  };
  critical.input.addEventListener("input", updatePreview);
  updatePreview();

  const fastLabel = el("label", "switch");
  const fast = el("input");
  fast.type = "checkbox";
  fast.checked = settings.warn_fast_polling;
  fastLabel.append(fast, el("span", undefined, "Warn when a provider refreshes faster than once a minute"));
  const trayLabel = el("label", "switch");
  const closeToTray = el("input");
  closeToTray.type = "checkbox";
  closeToTray.checked = settings.close_to_tray;
  trayLabel.append(closeToTray, el("span", undefined, "Close to system tray"));

  const errorEl = el("div", "form-error");
  errorEl.hidden = true;

  errorEl.setAttribute("role", "alert");
  const savePreferences = () => {
    if (!critical.input.reportValidity()) return;
    const preferences = {
      thresholds: { critical: Number(critical.input.value) },
      warn_fast_polling: fast.checked,
      close_to_tray: closeToTray.checked,
    };
    critical.input.disabled = fast.disabled = closeToTray.disabled = true;
    errorEl.hidden = true;
    preferencesSave = preferencesSave.then(async () => {
      try {
        const saved = await invoke<Settings>("save_preferences", { preferences });
        settings.thresholds = saved.thresholds;
        settings.warn_fast_polling = saved.warn_fast_polling;
        settings.close_to_tray = saved.close_to_tray;
      } catch (error) {
        critical.input.value = String(settings.thresholds.critical);
        fast.checked = settings.warn_fast_polling;
        closeToTray.checked = settings.close_to_tray;
        updatePreview();
        errorEl.textContent = `Could not save settings: ${String(error)}`;
        errorEl.hidden = false;
      } finally {
        critical.input.disabled = fast.disabled = closeToTray.disabled = false;
      }
    });
  };
  critical.input.addEventListener("change", savePreferences);
  fast.addEventListener("change", savePreferences);
  closeToTray.addEventListener("change", savePreferences);
  form.addEventListener("submit", event => {
    event.preventDefault();
    savePreferences();
  });

  const sectionCard = (title: string, card: HTMLElement) => {
    const section = el("section", "settings-group");
    section.setAttribute("aria-label", title);
    section.append(el("h2", undefined, title), card);
    return section;
  };
  const general = el("div", "settings-section general-settings");
  const colors = el("div", "settings-option");
  colors.append(el("h3", undefined, "Bar colors"), critical.field, colorHelp, preview, previewLabels);
  const polling = el("div", "settings-option");
  polling.append(el("h3", undefined, "Refresh warnings"), fastLabel);
  const trayOption = el("div", "settings-option");
  trayOption.append(el("h3", undefined, "System tray"), trayLabel,
    el("p", "setting-help", "Closing keeps monitoring running. Use Quit in the tray menu to exit. Choose tray entries inside each provider�s configuration."));
  general.append(colors, polling, trayOption, cliSettings());
  const providers = el("div", "settings-section");
  for (const entry of settings.providers) {
    const button = el("button", "provider-entry", providerName(entry.id));
    button.type = "button";
    if (entry.name) button.append(el("span", "setting-help", configLabel(entry.config)));
    button.addEventListener("click", () => renderEntryEditor(entry));
    providers.append(button);
  }
  const add = iconButton("plus", "Add provider", () => renderEntryEditor());
  add.classList.add("add-provider");
  providers.append(add);
  const about = el("div", "settings-section about");
  const repository = el("a", undefined, "GitHub repository");
  repository.href = meta.repository;
  repository.addEventListener("click", async event => {
    event.preventDefault();
    try { await invoke("open_repository"); }
    catch (error) { errorEl.textContent = String(error); errorEl.hidden = false; }
  });
  about.append(
    el("div", undefined, `Author: ${meta.author}`),
    el("div", undefined, `Version: ${meta.version}`),
    repository,
  );
  general.append(errorEl);
  form.append(sectionCard("General", general), sectionCard("Providers", providers), sectionCard("About", about));
  contentEl.replaceChildren(form);
}

function textField(label: string, value: string, placeholder = "") {
  const field = el("label", "field");
  const input = el("input"); input.type = "text"; input.value = value;
  input.placeholder = placeholder; input.spellcheck = false;
  field.append(el("span", undefined, label), input);
  return { field, input };
}
function selectField(label: string, choices: [string, string][], value: string) {
  const field = el("label", "field"); const input = el("select");
  for (const [key, name] of choices) { const option = el("option", undefined, name); option.value = key; input.append(option); }
  input.value = value; field.append(el("span", undefined, label), input); return { field, input };
}
function renderEntryEditor(entry?: ProviderSettings): void {
  const form = el("form", "settings");
  const section = el("section", "settings-section");
  const name = textField("Name (optional)", entry?.name ?? "", "Use provider and strategy name");
  const type = selectField("Provider", [["codex", "Codex"], ["claude", "Claude"]], entry?.config.kind === "claude" ? "claude" : "codex");
  const strategy = selectField("Strategy", [["inference", "Inference"], ["account", "App Server"]], entry?.config.kind === "codex_account" ? "account" : "inference");
  const strategyHelp = el("p", "setting-help");
  strategyHelp.id = "codex-strategy-help";
  strategyHelp.setAttribute("aria-live", "polite");
  strategy.input.setAttribute("aria-describedby", strategyHelp.id);
  const config = entry?.config;
  const model = textField("Model", config?.kind === "codex_inference" ? config.options.model : "gpt-6-luna");
  const effort = selectField("Effort", meta.codex_efforts.map(v => [v,v]), config?.kind === "codex_inference" ? config.options.effort : "low");
  const auth = textField("Auth.json file", config?.kind === "codex_inference" ? config.options.auth_file ?? "" : "", "Default Codex credentials");
  const executable = textField("Codex executable", config?.kind === "codex_account" ? config.options.executable : "codex");
  const home = textField("Codex home directory", config?.kind === "codex_account" ? config.options.codex_home ?? "" : "", "Default CODEX_HOME or ~/.codex");
  const credentials = textField("Credentials file", config?.kind === "claude" ? config.options.credentials_file ?? "" : "", "Default ~/.claude/.credentials.json");
  const trayControls = el("div", "settings-option");
  const trayLabel = el("label", "switch");
  const trayEnabled = el("input"); trayEnabled.type = "checkbox"; trayEnabled.checked = entry?.tray_enabled ?? false;
  trayLabel.append(trayEnabled, el("span", undefined, "Show in system tray"));
  const trayGroup = selectField("Tray quota group", [], "");
  if (entry) {
    trayGroup.input.dataset.trayEntry = entry.id;
    populateTrayGroups(trayGroup.input, entry);
  } else { trayGroup.field.hidden = true; }
  trayGroup.input.addEventListener("change", () => { trayGroup.input.dataset.trayDirty = "true"; });
  trayControls.append(el("h3", undefined, "System tray"), trayLabel, trayGroup.field,
    el("p", "setting-help", "Outer ring: 5 hours. Inner ring: 7 days. Keeps monitoring when hidden from the main window, using this entry�s refresh settings."));
  const fields = el("div", "settings");
  const updateFields = () => {
    strategy.field.hidden = type.input.value !== "codex";
    strategyHelp.hidden = type.input.value !== "codex";
    strategyHelp.textContent = strategy.input.value === "inference"
      ? "Inference sends a small request using the selected model and effort, then reads usage limits from the response. Each refresh consumes some quota and uses the selected auth.json credentials. It can report different limits from App Server."
      : "App Server reads account usage limits through the official Codex CLI without making an inference request. It requires Codex to be installed and logged in for the selected home directory. It can report different limits from Inference.";
    if (type.input.value === "claude") fields.replaceChildren(credentials.field);
    else if (strategy.input.value === "inference") fields.replaceChildren(auth.field, model.field, effort.field);
    else fields.replaceChildren(executable.field, home.field);
  };
  type.input.addEventListener("change", updateFields); strategy.input.addEventListener("change", updateFields); updateFields();
  const error = el("div", "form-error"); error.setAttribute("role", "alert");
  const actions = el("div", "actions");
  const cancel = el("button", undefined, "Cancel"); cancel.type = "button"; cancel.onclick = () => renderSettings();
  const save = el("button", "primary", "Save"); save.type = "submit";
  if (entry) {
    const remove = el("button", undefined, "Remove"); remove.type = "button";
    remove.onclick = async () => {
      remove.disabled = true;
      try { settings = await invoke<Settings>("remove_entry", {id: entry.id}); cardStates.delete(entry.id); render(); }
      catch (e) { error.textContent = String(e); remove.disabled = false; }
    };
    actions.append(remove);
  }
  actions.append(cancel, save);
  form.onsubmit = async event => {
    event.preventDefault(); save.disabled = true;
    const clean = (value: string) => value.trim() || null;
    let config: Configuration;
    if (type.input.value === "claude") config = {kind:"claude", options:{credentials_file:clean(credentials.input.value)}};
    else if (strategy.input.value === "account") config = {kind:"codex_account", options:{executable:executable.input.value.trim(), codex_home:clean(home.input.value),timeout_seconds:30}};
    else config = {kind:"codex_inference", options:{model:model.input.value.trim(),effort:effort.input.value,auth_file:clean(auth.input.value)}};
    try {
      const before = new Map(settings.providers.map(p => [p.id, p.generation]));
      settings = await invoke<Settings>("save_entry", {entry:{id:entry?.id ?? null,name:clean(name.input.value),config},tray:{enabled:trayEnabled.checked,group:trayGroup.input.value || null}});
      for (const p of settings.providers) {
        if (!before.has(p.id) || before.get(p.id) !== p.generation) {
          cardStates.delete(p.id);
          if (p.shown || p.tray_enabled) void invoke("refresh", {provider:p.id});
        }
      }
      render();
    } catch (e) { error.textContent = String(e); save.disabled = false; }
  };
  section.append(el("h2", undefined, entry ? "Edit provider" : "Add provider"), name.field, type.field, strategy.field, strategyHelp, fields, trayControls);
  form.append(section, error, actions); contentEl.replaceChildren(form);
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
  document.documentElement.style.setProperty("--available-height", `${Math.max(240, (window.screen.availHeight || 800) - 48)}px`);
  document.addEventListener("contextmenu", (event) => event.preventDefault());

  [meta, settings] = await Promise.all([
    invoke<Meta>("get_meta"),
    invoke<Settings>("get_settings"),
  ]);
  buildTitlebar();
  render();

  new ResizeObserver(() => void fitHeight()).observe(appEl);
  window.addEventListener("resize", () => void fitHeight());

  await listen<EntryReport>("usage-report", (event) => {
    const payload = event.payload;
    const entry = settings.providers.find(p => p.id === payload.entry_id);
    if (!entry || entry.generation !== payload.generation) return;
    const state = cardStates.get(entry.id) ?? { good: null, error: null };
    if (payload.source_report) {
      const source = payload.source_report;
      const errors = [source.error, ...source.groups.map(g => g.error)].filter(Boolean);
      state.error = errors.join("; ") || null;
      if (!source.error) {
        state.good = {
          provider: entry.id, plan: null, fetched_at: source.fetched_at,
          groups: source.groups,
          error: null, details: {},
          limit_reached: source.groups.some(g => g.ordinary_usage_allowed === false || g.spend_control_reached === true || g.rate_limit_reached_type !== null),
          metrics: source.groups.flatMap(g => g.metrics.map(metric => ({...metric, label: `${g.limit_name ?? g.limit_id ?? g.key} · ${metric.label}`}))),
        };
      }
    } else if (payload.report) {
      state.error = payload.report.error;
      if (!payload.report.error) state.good = payload.report;
    }
    cardStates.set(entry.id, state);
    for (const select of contentEl.querySelectorAll<HTMLSelectElement>("select[data-tray-entry]")) {
      if (select.dataset.trayEntry === entry.id && !select.disabled && !select.dataset.trayDirty) populateTrayGroups(select, entry);
    }
    updateCard(entry.id);
  });

  // The backend waits for this before fetching, so no report is missed.
  await invoke("start");
}

void init();
