"use strict";
// The browser renders and collects input. Rust owns decimal arithmetic, validation and persistence.
// The only arithmetic here is exact BigInt decimal totals for live input feedback, and pixel
// positions for charts.
const $ = (s) => document.querySelector(s);
const VIEWS = [
  ["home", "ホーム"],
  ["input", "かんたん入力"],
  ["journal", "仕訳帳"],
  ["accounts", "勘定科目・取引先"],
  ["claims", "債権・債務"],
  ["budget", "予算"],
  ["plans", "計画"],
  ["cost", "原価管理"],
  ["stats", "統計・予測"],
  ["reports", "帳票"],
  ["audit", "変更履歴"],
];
const S = {
  csrf: "",
  user: "",
  readonly: false,
  meta: null,
  books: [],
  book: null,
  view: "home",
  sub: {},
  quickKind: "expense",
};
const today = () => {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
};
const addMonths = (iso, n) => {
  const [y, m] = iso.split("-").map(Number);
  const t = y * 12 + (m - 1) + n;
  return `${Math.floor(t / 12)}-${String((t % 12) + 1).padStart(2, "0")}-01`;
};
const monthEnd = (iso) => {
  const [y, m] = iso.split("-").map(Number);
  const d = new Date(Date.UTC(y, m, 0)).getUTCDate();
  return `${y}-${String(m).padStart(2, "0")}-${d}`;
};

// ---------- DOM helpers ----------
function h(tag, props = {}, ...children) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(props || {})) {
    if (v === undefined || v === null || v === false) continue;
    if (k === "class") e.className = v;
    else if (k === "text") e.textContent = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else if (k === "value") e.value = v;
    else if (k === "checked") e.checked = v;
    else if (k === "style") {
      // CSP forbids style attributes; the CSSOM is allowed.
      for (const decl of String(v).split(";")) {
        const i = decl.indexOf(":");
        if (i > 0) e.style.setProperty(decl.slice(0, i).trim(), decl.slice(i + 1).trim());
      }
    } else e.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat(Infinity)) {
    if (c === null || c === undefined || c === false) continue;
    e.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return e;
}
const SVG = "http://www.w3.org/2000/svg";
function s(tag, attrs = {}, ...children) {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else e.setAttribute(k, v);
  }
  for (const c of children.flat())
    if (c) e.append(c instanceof Node ? c : document.createTextNode(c));
  return e;
}
/** replaceChildren that flattens arrays and skips empty values. */
function put(el, ...children) {
  el.replaceChildren(
    ...children.flat(Infinity).filter((c) => c !== null && c !== undefined && c !== false),
  );
}
function message(text, error = false) {
  const m = $("#message");
  m.textContent = text;
  m.className = error ? "error" : "";
  m.hidden = !text;
  if (text && !error) setTimeout(() => m.textContent === text && (m.hidden = true), 4000);
}

// ---------- Decimal (exact, BigInt fixed point) ----------
const SCALE = 10n ** 12n;
function ascii(text) {
  return String(text ?? "")
    .replace(/[０-９]/g, (c) => String.fromCharCode(c.charCodeAt(0) - 0xfee0))
    .replace(/．/g, ".")
    .replace(/[－−]/g, "-")
    .replace(/[,，_\s¥￥円]/g, "");
}
function dec(text) {
  let t = ascii(text);
  if (t === "") return 0n;
  if (!/^-?\d+(\.\d{1,12})?$/.test(t)) return null;
  const neg = t.startsWith("-");
  if (neg) t = t.slice(1);
  const [i, f = ""] = t.split(".");
  const v = BigInt(i) * SCALE + BigInt((f + "0".repeat(12)).slice(0, 12));
  return neg ? -v : v;
}
function decText(v) {
  const neg = v < 0n;
  if (neg) v = -v;
  const f = (v % SCALE).toString().padStart(12, "0").replace(/0+$/, "");
  return (neg ? "-" : "") + (v / SCALE).toString() + (f ? "." + f : "");
}
/** Thousands separators for a decimal string; never converts to a float. */
function fmt(v) {
  if (v === null || v === undefined || v === "") return "";
  const t = String(v);
  const neg = t.startsWith("-");
  const [i, f] = (neg ? t.slice(1) : t).split(".");
  return (neg ? "-" : "") + i.replace(/\B(?=(\d{3})+(?!\d))/g, ",") + (f ? "." + f : "");
}
function num(v, cls) {
  const t = String(v ?? "");
  return h(
    "td",
    { class: "num" + (t.startsWith("-") ? " neg" : "") + (cls ? " " + cls : "") },
    fmt(t),
  );
}
const isZero = (v) => dec(v) === 0n;

// ---------- API ----------
async function post(path, body) {
  const r = await fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json", "x-yy-csrf": S.csrf },
    body: JSON.stringify(body || {}),
  });
  const data = await r.json().catch(() => ({}));
  if (!r.ok) {
    const e = new Error(data.error || `HTTP ${r.status}`);
    e.status = r.status;
    throw e;
  }
  return data;
}
const api = (tool, args = {}) => post("/web/api", { tool, arguments: args });
const bookArgs = (extra = {}) => ({ book_id: S.book.id, ...extra });
async function change(operation, done = "保存しました") {
  try {
    const r = await api("book_change", {
      book_id: S.book.id,
      expected_revision: S.book.revision,
      change: operation,
    });
    await loadBook();
    message(done);
    return r;
  } catch (e) {
    if (e.status === 409) {
      await loadBook();
      render();
    }
    throw e;
  }
}

// ---------- Session ----------
async function boot() {
  $("#login-form").addEventListener("submit", async (ev) => {
    ev.preventDefault();
    try {
      const r = await fetch("/web/login", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ password: $("#password").value }),
      });
      const data = await r.json();
      if (!r.ok) throw new Error(data.error || "ログインできません");
      $("#password").value = "";
      await start(data);
    } catch (e) {
      $("#login-error").textContent = e.message;
    }
  });
  $("#logout").addEventListener("click", async () => {
    await post("/web/logout").catch(() => {});
    location.reload();
  });
  const r = await fetch("/web/session");
  if (r.ok) await start(await r.json());
  else $("#login").hidden = false;
}
async function start(session) {
  S.csrf = session.csrf;
  S.user = session.user;
  S.readonly = session.read_only;
  $("#login").hidden = true;
  $("#app").hidden = false;
  $("#user").textContent = S.user + (S.readonly ? "（閲覧のみ）" : "");
  S.meta = await api("metadata");
  for (const [key, label] of VIEWS) {
    $("#views").append(h("button", { "data-view": key, onclick: () => go(key) }, label));
  }
  $("#books").addEventListener("change", async () => {
    await loadBook(Number($("#books").value));
    render();
  });
  $("#new-book").addEventListener("click", newBook);
  $("#book-settings").addEventListener("click", bookSettings);
  for (const b of ["#new-book", "#book-settings"]) $(b).disabled = S.readonly;
  const hash = location.hash.slice(1);
  if (VIEWS.some(([k]) => k === hash)) S.view = hash;
  await loadBooks();
  render();
}
async function loadBooks(select) {
  S.books = (await api("book_list")).books;
  const sel = $("#books");
  put(
    sel,
    ...S.books.map((b) => h("option", { value: b.id }, `${b.name} · ${label("kinds", b.kind)}`)),
  );
  const id = select || S.book?.id || S.books[0]?.id;
  if (id) {
    sel.value = id;
    await loadBook(id);
  }
}
async function loadBook(id = S.book?.id) {
  S.book = await api("book_get", { book_id: id });
  $("#revision").textContent = `版 ${S.book.revision} · ${S.book.currency}`;
  $("#book-kind").textContent = `${label("kinds", S.book.kind)}の帳簿`;
}
function label(group, key) {
  return (S.meta?.[group] || []).find(([k]) => k === key)?.[1] ?? key ?? "";
}
function go(view) {
  S.view = view;
  history.replaceState(null, "", "#" + view);
  render();
}
async function render() {
  for (const b of document.querySelectorAll("#views button"))
    b.setAttribute("aria-current", b.dataset.view === S.view ? "page" : "false");
  $("#view-title").textContent = VIEWS.find(([k]) => k === S.view)[1];
  const surface = $("#surface");
  if (!S.book) {
    put(
      surface,
      h(
        "div",
        { class: "card" },
        h(
          "p",
          {},
          "帳簿がありません。「新規帳簿」から家計または個人事業主の帳簿を作成してください。",
        ),
      ),
    );
    return;
  }
  const seq = (S.renderSeq = (S.renderSeq || 0) + 1);
  surface.style.opacity = "0.6";
  try {
    const content = await RENDER[S.view]();
    // A slower, older render must not overwrite the view the user switched to.
    if (seq !== S.renderSeq) return;
    put(surface, content);
  } catch (e) {
    message(e.message, true);
  } finally {
    if (seq === S.renderSeq) surface.style.opacity = "";
  }
}

// ---------- Generic modal form ----------
function modal(title, body, { wide = false, submit = "保存", onSubmit, extra = [] } = {}) {
  const error = h("div", { class: "error", role: "alert" });
  const close = () => backdrop.remove();
  const save = h("button", { class: "primary", type: "submit" }, submit);
  const form = h(
    "form",
    {
      class: "modal" + (wide ? " wide" : ""),
      role: "dialog",
      "aria-modal": "true",
      "aria-label": title,
      onsubmit: async (ev) => {
        ev.preventDefault();
        if (!onSubmit) return close();
        save.disabled = true;
        error.textContent = "";
        try {
          if ((await onSubmit()) !== false) close();
        } catch (e) {
          error.textContent = e.message;
        } finally {
          save.disabled = false;
        }
      },
    },
    h("h2", {}, title),
    body,
    error,
    h(
      "div",
      { class: "actions" },
      extra,
      h("span", { class: "spacer" }),
      h("button", { type: "button", onclick: close }, onSubmit ? "キャンセル" : "閉じる"),
      onSubmit && !S.readonly ? save : null,
    ),
  );
  const backdrop = h(
    "div",
    { class: "modal-backdrop", onkeydown: (e) => e.key === "Escape" && close() },
    form,
  );
  $("#modal-host").append(backdrop);
  setTimeout(() => form.querySelector("input,select,textarea")?.focus(), 0);
  return { close, form };
}
/** fields: [key, label, type, options] -> {element, values()} */
function fields(spec) {
  const inputs = {};
  const grid = h("div", { class: "form" });
  for (const f of spec) {
    const { key, label: text, type = "text", value, options, help, wide } = f;
    let input;
    if (type === "select") {
      input = h(
        "select",
        { id: "f-" + key },
        options.map(([v, l]) => h("option", { value: v ?? "" }, l)),
      );
      input.value = value ?? "";
    } else if (type === "textarea") {
      input = h("textarea", { id: "f-" + key, value: value ?? "" });
    } else if (type === "checkbox") {
      input = h("input", { id: "f-" + key, type: "checkbox", checked: !!value });
    } else {
      input = h("input", {
        id: "f-" + key,
        type: type === "decimal" ? "text" : type,
        inputmode: type === "decimal" ? "decimal" : null,
        value: value ?? "",
        required: f.required,
        placeholder: f.placeholder,
      });
    }
    inputs[key] = { input, type };
    if (wide)
      grid.append(
        h("label", { class: "wide", for: "f-" + key }, text),
        h("div", { class: "wide" }, input),
      );
    else grid.append(h("label", { for: "f-" + key }, text), input);
    if (help) grid.append(h("div", { class: "help" }, help));
  }
  return {
    element: grid,
    inputs,
    values() {
      const out = {};
      for (const [k, { input, type }] of Object.entries(inputs)) {
        if (type === "checkbox") out[k] = input.checked;
        else if (type === "number") out[k] = input.value === "" ? null : Number(input.value);
        else if (type === "decimal") out[k] = input.value.trim() === "" ? null : ascii(input.value);
        else if (type === "id") out[k] = input.value === "" ? null : Number(input.value);
        else out[k] = input.value;
      }
      return out;
    },
  };
}
const idOrNull = (v) => (v === "" || v === null || v === undefined ? null : Number(v));

// ---------- Book lookups ----------
const account = (id) => S.book.accounts.find((a) => a.id === id);
const accountName = (id) => account(id)?.name ?? `#${id}`;
const partnerName = (id) => S.book.partners.find((p) => p.id === id)?.name ?? "";
function accountOptions(filter = () => true, blank) {
  const opts = blank ? [["", blank]] : [];
  for (const [cat, catLabel] of S.meta.categories) {
    for (const a of S.book.accounts
      .filter((a) => a.category === cat && !a.archived && filter(a))
      .sort((a, b) => a.code.localeCompare(b.code)))
      opts.push([a.id, `${catLabel}｜${a.code} ${a.name}`]);
  }
  return opts;
}
function accountSelect(filter, value, blank) {
  const sel = h(
    "select",
    {},
    accountOptions(filter, blank).map(([v, l]) => h("option", { value: v }, l)),
  );
  if (value !== undefined && value !== null) sel.value = value;
  // A remembered value that is not offered here (other book, archived) falls back to the first option.
  if (sel.selectedIndex < 0) sel.selectedIndex = 0;
  return sel;
}
const partnerOptions = () => [
  ["", "（なし）"],
  ...S.book.partners.filter((p) => !p.archived).map((p) => [p.id, p.name]),
];
const isBusiness = () => S.book.kind === "sole_proprietor";
const fyStart = (date = today()) => {
  const [y, m] = date.split("-").map(Number);
  const start = S.book.fiscal_start_month;
  return m >= start ? y : y - 1;
};

// ---------- Charts ----------
function tooltip(ev, rows, title) {
  const t = $("#tooltip");
  put(
    t,
    h("div", { class: "muted" }, title),
    ...rows.map(([name, value, color, dash]) =>
      h(
        "div",
        { class: "row" },
        h("span", { class: "key", style: `background:${color};${dash ? "opacity:.6" : ""}` }),
        h("strong", {}, fmt(value)),
        h("span", {}, name),
      ),
    ),
  );
  t.hidden = false;
  const x = Math.min(ev.clientX + 14, innerWidth - t.offsetWidth - 8);
  t.style.left = x + "px";
  t.style.top = ev.clientY + 14 + "px";
}
const hideTip = () => ($("#tooltip").hidden = true);
function niceMax(v) {
  if (v <= 0) return 1;
  const p = 10 ** Math.floor(Math.log10(v));
  return [1, 2, 2.5, 5, 10].map((m) => m * p).find((m) => m >= v);
}
const compact = (v) =>
  Math.abs(v) >= 1e6 ? `${v / 1e6}M` : Math.abs(v) >= 1e3 ? `${v / 1e3}K` : `${v}`;
/** Grouped income/expense columns. Values are decimal strings; Number() is used only for pixels. */
function barChart(months) {
  const W = 720,
    H = 240,
    L = 48,
    B = 24,
    T = 10;
  const max = niceMax(Math.max(...months.flatMap((m) => [Number(m.income), Number(m.expense)]), 0));
  const y = (v) => T + (H - T - B) * (1 - Number(v) / max);
  const band = (W - L) / months.length;
  const bw = Math.min(24, (band - 10) / 2);
  const svg = s("svg", {
    class: "chart",
    viewBox: `0 0 ${W} ${H}`,
    role: "img",
    "aria-label": "月別の収入と支出",
  });
  for (let i = 0; i <= 4; i++) {
    const v = (max / 4) * i;
    svg.append(
      s("line", { class: "grid", x1: L, x2: W, y1: y(v), y2: y(v) }),
      s("text", { class: "axis", x: L - 6, y: y(v) + 4, "text-anchor": "end" }, compact(v)),
    );
  }
  months.forEach((m, i) => {
    const x0 = L + band * i + band / 2 - bw - 1;
    for (const [j, key] of [
      [0, "income"],
      [1, "expense"],
    ]) {
      const v = Math.max(0, Number(m[key]));
      const top = y(v),
        hgt = Math.max(0, H - B - top);
      const r = Math.min(4, hgt);
      const x = x0 + j * (bw + 2);
      svg.append(
        s("path", {
          class: "bar " + key,
          d: `M${x},${H - B}V${top + r}Q${x},${top} ${x + r},${top}H${x + bw - r}Q${x + bw},${top} ${x + bw},${top + r}V${H - B}Z`,
        }),
      );
    }
    const hit = s("rect", {
      class: "hit",
      x: L + band * i,
      y: T,
      width: band,
      height: H - T - B,
      tabindex: 0,
      onpointermove: (ev) =>
        tooltip(
          ev,
          [
            ["収入", m.income, "var(--income)"],
            ["支出", m.expense, "var(--expense)"],
            ["収支", m.net, "var(--text-2)"],
          ],
          m.month,
        ),
      onpointerleave: hideTip,
      onfocus: (ev) => {
        const r = ev.target.getBoundingClientRect();
        tooltip(
          { clientX: r.left, clientY: r.top },
          [
            ["収入", m.income, "var(--income)"],
            ["支出", m.expense, "var(--expense)"],
          ],
          m.month,
        );
      },
      onblur: hideTip,
    });
    svg.append(hit);
    if (months.length <= 24 && (months.length <= 12 || i % 2 === 0))
      svg.append(
        s(
          "text",
          { class: "axis", x: L + band * i + band / 2, y: H - 6, "text-anchor": "middle" },
          m.month.slice(2).replace("-", "/"),
        ),
      );
  });
  return h(
    "div",
    {},
    h(
      "div",
      { class: "legend" },
      h("span", {}, h("i", { style: "background:var(--income)" }), "収入"),
      h("span", {}, h("i", { style: "background:var(--expense)" }), "支出"),
    ),
    svg,
  );
}
/** History of total expense (solid) and forecast (dashed) with an approximately 95% band. */
function forecastChart(history, f) {
  const W = 720,
    H = 240,
    L = 56,
    B = 24,
    T = 12;
  const points = [
    ...history.map((m) => ({ month: m.month, value: m.expense, kind: "actual" })),
    ...f.totals.map((t) => ({
      month: t.month,
      value: t.expense,
      low: t.expense_low,
      high: t.expense_high,
      kind: "forecast",
    })),
  ];
  if (!points.length) return h("p", { class: "muted" }, "データがありません");
  const max = niceMax(Math.max(...points.map((p) => Number(p.high ?? p.value)), 0));
  const x = (i) => L + ((W - L - 12) * i) / Math.max(1, points.length - 1);
  const y = (v) => T + (H - T - B) * (1 - Number(v) / max);
  const svg = s("svg", {
    class: "chart",
    viewBox: `0 0 ${W} ${H}`,
    role: "img",
    "aria-label": "支出の実績と予測",
  });
  for (let i = 0; i <= 4; i++) {
    const v = (max / 4) * i;
    svg.append(
      s("line", { class: "grid", x1: L, x2: W, y1: y(v), y2: y(v) }),
      s("text", { class: "axis", x: L - 6, y: y(v) + 4, "text-anchor": "end" }, compact(v)),
    );
  }
  const fi = points.findIndex((p) => p.kind === "forecast");
  if (fi >= 0) {
    const fp = points.slice(fi);
    const top = fp.map((p, k) => `${x(fi + k)},${y(p.high)}`).join(" ");
    const bottom = fp
      .map((p, k) => `${x(fi + k)},${y(p.low)}`)
      .reverse()
      .join(" ");
    svg.append(s("polygon", { class: "band", points: `${top} ${bottom}` }));
  }
  const actual = points.filter((p) => p.kind === "actual");
  if (actual.length)
    svg.append(
      s("polyline", {
        class: "line-expense",
        points: actual.map((p, i) => `${x(i)},${y(p.value)}`).join(" "),
      }),
    );
  if (fi >= 0) {
    const start = Math.max(0, fi - 1);
    svg.append(
      s("polyline", {
        class: "line-forecast",
        points: points
          .slice(start)
          .map((p, k) => `${x(start + k)},${y(p.value)}`)
          .join(" "),
      }),
    );
  }
  const last = points.length - 1;
  svg.append(s("circle", { class: "dot", cx: x(last), cy: y(points[last].value), r: 4 }));
  const cross = s("line", { class: "cross", y1: T, y2: H - B, x1: -10, x2: -10 });
  svg.append(cross);
  points.forEach((p, i) => {
    if (points.length <= 24 && (points.length <= 12 || i % 3 === 0))
      svg.append(
        s(
          "text",
          { class: "axis", x: x(i), y: H - 6, "text-anchor": "middle" },
          p.month.slice(2).replace("-", "/"),
        ),
      );
  });
  svg.append(
    s("rect", {
      class: "hit",
      x: L,
      y: T,
      width: W - L,
      height: H - T - B,
      onpointermove: (ev) => {
        const r = svg.getBoundingClientRect();
        const px = ((ev.clientX - r.left) / r.width) * W;
        const i = Math.max(0, Math.min(last, Math.round(((px - L) / (W - L - 12)) * last)));
        cross.setAttribute("x1", x(i));
        cross.setAttribute("x2", x(i));
        const p = points[i];
        tooltip(
          ev,
          p.kind === "actual"
            ? [["支出実績", p.value, "var(--expense)"]]
            : [
                ["支出予測", p.value, "var(--expense)", true],
                ["上限", p.high, "var(--muted)"],
                ["下限", p.low, "var(--muted)"],
              ],
          p.month,
        );
      },
      onpointerleave: () => {
        hideTip();
        cross.setAttribute("x1", -10);
        cross.setAttribute("x2", -10);
      },
    }),
  );
  return h(
    "div",
    {},
    h(
      "div",
      { class: "legend" },
      h("span", {}, h("i", { class: "line", style: "background:var(--expense)" }), "支出実績"),
      h("span", {}, h("i", { class: "dash" }), "支出予測"),
      h(
        "span",
        {},
        h("i", { style: "background:var(--expense);opacity:.25" }),
        "予測の幅（約95%）",
      ),
    ),
    svg,
  );
}
function meter(actual, budget) {
  const a = Number(actual),
    b = Number(budget);
  const pct = b > 0 ? Math.min(100, (a / b) * 100) : 0;
  return h(
    "div",
    { class: "meter" + (b > 0 && a > b ? " over" : ""), title: `${fmt(actual)} / ${fmt(budget)}` },
    h("span", { style: `width:${pct}%` }),
  );
}
function table(head, rows, { numeric = [] } = {}) {
  return h(
    "div",
    { class: "scroll" },
    h(
      "table",
      {},
      h(
        "thead",
        {},
        h(
          "tr",
          {},
          head.map((t, i) => h("th", { class: numeric.includes(i) ? "num" : null }, t)),
        ),
      ),
      h("tbody", {}, rows),
    ),
  );
}
function tabs(key, items, onChange) {
  const current = S.sub[key] || items[0][0];
  return h(
    "div",
    { class: "subtabs", role: "tablist" },
    items.map(([k, l]) =>
      h(
        "button",
        {
          role: "tab",
          "aria-selected": k === current ? "true" : "false",
          onclick: () => ((S.sub[key] = k), onChange ? onChange(k) : render()),
        },
        l,
      ),
    ),
  );
}

// ---------- Views ----------
const RENDER = {};

RENDER.home = async () => {
  const t = today();
  const monthStart = t.slice(0, 8) + "01";
  const from = addMonths(monthStart, -11);
  const [stats, fc, cal, claims] = await Promise.all([
    api("statistics", bookArgs({ from, to: monthEnd(t) })),
    api("forecast", bookArgs({ as_of: t, months: 3 })),
    api(
      "plan_calendar",
      bookArgs({ from: addMonths(monthStart, -12), to: addMonths(monthStart, 1) }),
    ),
    api("claim_report", bookArgs({ as_of: t })),
  ]);
  const cur = stats.months[stats.months.length - 1];
  const due = cal.occurrences.filter((o) => !o.posted && o.date <= monthEnd(t));
  const overdue = claims.claims.filter((c) => !isZero(c.arrears));
  const tiles = h(
    "div",
    { class: "tiles" },
    tile("今月の収入", cur.income),
    tile("今月の支出", cur.expense),
    tile("今月の収支", cur.net, cur.savings_rate ? `貯蓄率 ${cur.savings_rate}%` : ""),
    tile("過去12か月の平均支出", stats.average_expense, `平均収入 ${fmt(stats.average_income)}`),
    tile(
      "来月の支出予想",
      fc.totals[0]?.expense ?? "0",
      fc.totals[0] ? `幅 ${fmt(fc.totals[0].expense_low)}〜${fmt(fc.totals[0].expense_high)}` : "",
    ),
  );
  const budgets = S.book.budgets.filter((b) => b.fiscal_year === fyStart());
  let budgetCard = null;
  if (budgets.length) {
    const r = await api("budget_report", bookArgs({ budget_id: budgets[0].id }));
    const mi = r.months.indexOf(t.slice(0, 7));
    if (mi >= 0)
      budgetCard = h(
        "div",
        { class: "card" },
        h("h2", {}, `今月の予算消化（${r.name}）`),
        table(
          ["科目", "予算", "実績", "", "残り"],
          r.rows
            .filter((x) => x.category === "expense" && !x.unbudgeted)
            .map((x) =>
              h(
                "tr",
                {},
                h("td", {}, x.name),
                num(x.budget[mi]),
                num(x.actual[mi]),
                h("td", {}, meter(x.actual[mi], x.budget[mi])),
                num(decText(dec(x.budget[mi]) - dec(x.actual[mi]))),
              ),
            ),
          { numeric: [1, 2, 4] },
        ),
      );
  }
  return h(
    "div",
    {},
    tiles,
    h("div", { class: "card" }, h("h2", {}, "過去12か月の収入と支出"), barChart(stats.months)),
    h(
      "div",
      { class: "grid2" },
      h(
        "div",
        { class: "card" },
        h("h2", {}, "計上待ちの計画"),
        due.length
          ? table(
              ["予定日", "計画", "金額", ""],
              due
                .slice(0, 12)
                .map((o) =>
                  h(
                    "tr",
                    {},
                    h("td", {}, o.date),
                    h("td", {}, o.name),
                    num(o.amount),
                    h("td", {}, postButton(o)),
                  ),
                ),
              { numeric: [2] },
            )
          : h("p", { class: "muted" }, "今月までの計画はすべて計上済みです。"),
      ),
      h(
        "div",
        { class: "card" },
        h("h2", {}, "延滞している債権・債務"),
        overdue.length
          ? table(
              ["件名", "区分", "延滞額", "経過"],
              overdue.map((c) =>
                h(
                  "tr",
                  {},
                  h("td", {}, c.title),
                  h("td", {}, c.side === "receivable" ? "債権" : "債務"),
                  num(c.arrears),
                  h("td", {}, `${c.days_overdue}日`),
                ),
              ),
              { numeric: [2] },
            )
          : h("p", { class: "muted" }, "延滞はありません。"),
      ),
    ),
    budgetCard,
  );
};
function tile(labelText, value, sub = "") {
  return h(
    "div",
    { class: "tile" },
    h("div", { class: "label" }, labelText),
    h("div", { class: "value" + (String(value).startsWith("-") ? " neg" : "") }, fmt(value)),
    h("div", { class: "sub" }, sub),
  );
}
function postButton(o) {
  return h(
    "button",
    {
      disabled: S.readonly,
      onclick: () => {
        const f = fields([
          { key: "date", label: "計上日", type: "date", value: o.date },
          {
            key: "amount",
            label: "金額",
            type: "decimal",
            value: o.amount,
            help: "実際の金額が予定と異なる場合は変更してください",
          },
        ]);
        modal(
          `「${o.name}」を計上`,
          h(
            "div",
            {},
            h("p", {}, `${accountName(o.debit)} ／ ${accountName(o.credit)}`),
            f.element,
          ),
          {
            submit: "計上",
            onSubmit: async () => {
              const v = f.values();
              await change(
                {
                  operation: "post_plan",
                  plan: o.plan,
                  occurrence: o.date,
                  date: v.date,
                  amount: v.amount,
                },
                "計上しました",
              );
              render();
            },
          },
        );
      },
    },
    "計上",
  );
}

// ----- Quick input -----
RENDER.input = async () => {
  const kind = S.quickKind;
  const book = S.book;
  const accountFilter = {
    expense: (a) => a.category === "expense" || (a.category === "asset" && !a.payment),
    income: (a) => a.category === "income",
    transfer: (a) => ["asset", "liability", "equity"].includes(a.category),
  }[kind];
  const viaFilter = (a) =>
    a.payment ||
    (kind !== "transfer" && ["owner_contribution", "owner_draw"].includes(a.role)) ||
    (kind === "transfer" && ["asset", "liability", "equity"].includes(a.category));
  const accountSel = accountSelect(accountFilter, S.lastAccount?.[`${S.book.id}:${kind}`]);
  const via = accountSelect(viaFilter, S.lastVia?.[`${S.book.id}:${kind}`]);
  const date = h("input", { type: "date", value: S.lastDate || today() });
  const amount = h("input", { inputmode: "decimal", placeholder: "例: 1980", class: "amount" });
  const desc = h("input", { placeholder: "例: スーパーで食材" });
  const partner = h(
    "select",
    {},
    partnerOptions().map(([v, l]) => h("option", { value: v }, l)),
  );
  const ratio = h("input", { inputmode: "decimal", placeholder: "100", value: "" });
  const memo = h("input", {});
  const guide = h("div", { class: "guide" });
  const search = h("input", {
    type: "search",
    placeholder: "何に使いましたか？（例: ガソリン、打合せのコーヒー、住民税）",
  });
  const suggestions = h("div", { class: "suggest" });
  const showGuide = async () => {
    const a = account(Number(accountSel.value));
    if (!a) return put(guide);
    const g = await api("account_guide", bookArgs({ account: a.id }));
    put(
      guide,
      h("h4", {}, `${a.code} ${a.name} の使い方`),
      h("p", { class: "pre" }, a.guide || "説明は登録されていません。勘定科目画面で追加できます。"),
      g.distinctions.map((d) => distinction(d, (id) => ((accountSel.value = id), showGuide()))),
    );
  };
  accountSel.addEventListener("change", showGuide);
  let timer;
  search.addEventListener("input", () => {
    clearTimeout(timer);
    timer = setTimeout(async () => {
      const q = search.value.trim();
      if (!q) return put(suggestions);
      const g = await api("account_guide", bookArgs({ query: q }));
      put(
        suggestions,
        ...g.suggestions
          .filter(
            (x) =>
              x.score > 0 && [...accountSel.options].some((o) => o.value === String(x.account)),
          )
          .slice(0, 6)
          .map((x) =>
            h(
              "button",
              {
                type: "button",
                title: x.guide,
                onclick: () => ((accountSel.value = x.account), showGuide()),
              },
              `${x.name}`,
              h("small", {}, x.matched.slice(0, 2).join("・")),
            ),
          ),
      );
      if (!suggestions.children.length)
        suggestions.append(
          h("span", { class: "muted" }, "候補が見つかりません。下の科目一覧から選んでください。"),
        );
    }, 200);
  });
  const recent = await api("entry_list", bookArgs({ limit: 10 }));
  const labels = {
    expense: ["支出の科目", "支払方法"],
    income: ["収入の科目", "入金先"],
    transfer: ["移動先（増える方）", "移動元（減る方）"],
  }[kind];
  const save = async (ev) => {
    ev.preventDefault();
    const q = {
      kind,
      date: date.value,
      amount: ascii(amount.value),
      account: Number(accountSel.value),
      via: Number(via.value),
      description: desc.value,
      partner: idOrNull(partner.value),
      memo: memo.value,
    };
    if (kind === "expense" && isBusiness() && ratio.value.trim())
      q.business_percent = ascii(ratio.value);
    try {
      await change({ operation: "quick_entry", quick: q }, "記録しました");
      S.lastDate = date.value;
      S.lastAccount = { ...(S.lastAccount || {}), [`${S.book.id}:${kind}`]: q.account };
      S.lastVia = { ...(S.lastVia || {}), [`${S.book.id}:${kind}`]: q.via };
      render();
    } catch (e) {
      message(e.message, true);
    }
  };
  setTimeout(showGuide, 0);
  return h(
    "div",
    { class: "grid2" },
    h(
      "form",
      { class: "card", onsubmit: save },
      h(
        "div",
        { class: "kind-switch", role: "group", "aria-label": "取引の種類" },
        [
          ["expense", "支出"],
          ["income", "収入"],
          ["transfer", "振替"],
        ].map(([k, l]) =>
          h(
            "button",
            {
              type: "button",
              "aria-pressed": k === kind ? "true" : "false",
              onclick: () => ((S.quickKind = k), render()),
            },
            l,
          ),
        ),
      ),
      h(
        "p",
        { class: "muted" },
        {
          expense:
            "お金を使ったとき。カード払いは支払方法に「クレジットカード」を選び、引落し日は「振替」で記録します。",
          income: "給与・売上などお金を受け取ったとき。",
          transfer:
            "口座間の移動・ATM引出し・カード引落し・ローン元金の返済など、収入でも支出でもない移動。",
        }[kind],
      ),
      h(
        "div",
        { class: "form" },
        h("label", {}, "日付"),
        date,
        h("label", {}, "金額"),
        amount,
        kind !== "transfer"
          ? [h("label", {}, "科目を探す"), search, h("div", { class: "help" }, suggestions)]
          : null,
        h("label", {}, labels[0]),
        accountSel,
        h("label", {}, labels[1]),
        via,
        kind === "expense" && isBusiness()
          ? [
              h("label", {}, "事業割合（%）"),
              ratio,
              h(
                "div",
                { class: "help" },
                "自宅兼事務所の家賃・光熱費など。空欄は100%。残りは事業主貸になります。",
              ),
            ]
          : null,
        h("label", {}, "内容"),
        desc,
        h("label", {}, "取引先"),
        partner,
        h("label", {}, "メモ"),
        memo,
      ),
      h(
        "div",
        { class: "toolbar", style: "margin-top:14px" },
        h("button", { class: "primary", type: "submit", disabled: S.readonly }, "記録する"),
      ),
    ),
    h(
      "div",
      {},
      h("div", { class: "card" }, h("h2", {}, "科目の選び方"), guide),
      h("div", { class: "card" }, h("h2", {}, "最近の記録"), entryTable(recent.entries, true)),
    ),
  );
};
function distinction(d, pick) {
  return h(
    "div",
    { class: "distinction" },
    h("h4", {}, d.title),
    h("p", {}, d.explanation),
    h(
      "table",
      {},
      h(
        "tbody",
        {},
        d.examples.map(([example, id, name]) =>
          h(
            "tr",
            {},
            h("td", {}, example),
            h(
              "td",
              {},
              "→ ",
              pick && id
                ? h("button", { class: "link", type: "button", onclick: () => pick(id) }, name)
                : name,
            ),
          ),
        ),
      ),
    ),
  );
}

// ----- Journal -----
function entryTable(entries, compact = false) {
  if (!entries.length) return h("p", { class: "muted" }, "仕訳はありません。");
  const rows = [];
  for (const e of entries) {
    const debit = e.lines.filter((l) => !isZero(l.debit));
    const credit = e.lines.filter((l) => !isZero(l.credit));
    const n = Math.max(debit.length, credit.length);
    for (let i = 0; i < n; i++) {
      const d = debit[i],
        c = credit[i];
      rows.push(
        h(
          "tr",
          {
            class: (e.void ? "void " : "") + (compact ? "" : "clickable"),
            onclick: compact ? null : () => editEntry(e),
          },
          i === 0
            ? [
                h("td", { rowspan: n, class: "nowrap" }, e.date),
                compact ? null : h("td", { rowspan: n, class: "num" }, e.id),
                h(
                  "td",
                  { rowspan: n },
                  e.description,
                  e.void ? h("span", { class: "badge bad" }, "取消") : null,
                  e.source ? h("span", { class: "badge" }, sourceLabel(e.source)) : null,
                  e.partner ? h("div", { class: "muted" }, partnerName(e.partner)) : null,
                ),
              ]
            : null,
          h("td", { class: "nowrap" }, d ? accountName(d.account) : ""),
          d ? num(d.debit) : h("td"),
          h("td", { class: "nowrap" }, c ? accountName(c.account) : ""),
          c ? num(c.credit) : h("td"),
        ),
      );
    }
  }
  return table(
    compact
      ? ["日付", "内容", "借方", "金額", "貸方", "金額"]
      : ["日付", "No.", "摘要", "借方科目", "借方金額", "貸方科目", "貸方金額"],
    rows,
    { numeric: compact ? [3, 5] : [1, 4, 6] },
  );
}
function sourceLabel(source) {
  return source.startsWith("plan:")
    ? "計画"
    : source.startsWith("settle:")
      ? "回収・返済"
      : source.startsWith("claim:")
        ? "債権債務"
        : source.startsWith("depreciation:")
          ? "減価償却"
          : "自動";
}
RENDER.journal = async () => {
  const f = (S.journalFilter ||= {
    from: addMonths(today().slice(0, 8) + "01", -2),
    to: "",
    account: "",
    query: "",
    include_void: false,
    offset: 0,
  });
  const args = bookArgs({ limit: 100, offset: f.offset, include_void: f.include_void });
  if (f.from) args.from = f.from;
  if (f.to) args.to = f.to;
  if (f.account) args.account = Number(f.account);
  if (f.query) args.query = f.query;
  const page = await api("entry_list", args);
  const bind = (el, key, prop = "value") => (
    el.addEventListener("change", () => ((f[key] = el[prop]), (f.offset = 0), render())), el
  );
  return h(
    "div",
    {},
    h(
      "div",
      { class: "toolbar" },
      h(
        "label",
        {},
        "期間",
        bind(h("input", { type: "date", value: f.from }), "from"),
        "〜",
        bind(h("input", { type: "date", value: f.to }), "to"),
      ),
      bind(
        accountSelect(() => true, f.account, "すべての科目"),
        "account",
      ),
      bind(
        h("input", { type: "search", placeholder: "摘要・取引先・金額で検索", value: f.query }),
        "query",
      ),
      h(
        "label",
        {},
        bind(h("input", { type: "checkbox", checked: f.include_void }), "include_void", "checked"),
        "取消を表示",
      ),
      h("span", { class: "spacer" }),
      h(
        "button",
        { class: "primary", disabled: S.readonly, onclick: () => editEntry(null) },
        "仕訳を追加",
      ),
    ),
    h(
      "div",
      { class: "card" },
      h(
        "div",
        { class: "muted", style: "margin-bottom:6px" },
        `${page.total}件中 ${Math.min(page.total, f.offset + 1)}〜${Math.min(page.total, f.offset + page.entries.length)}件 · 行をクリックで編集`,
      ),
      entryTable(page.entries),
      h(
        "div",
        { class: "toolbar", style: "margin-top:8px" },
        h(
          "button",
          {
            disabled: f.offset === 0,
            onclick: () => ((f.offset = Math.max(0, f.offset - 100)), render()),
          },
          "前へ",
        ),
        h(
          "button",
          { disabled: f.offset + 100 >= page.total, onclick: () => ((f.offset += 100), render()) },
          "次へ",
        ),
      ),
    ),
  );
};
function editEntry(entry) {
  const e = entry
    ? structuredClone(entry)
    : {
        id: 0,
        date: today(),
        description: "",
        partner: null,
        lines: [
          { account: "", debit: "", credit: "" },
          { account: "", debit: "", credit: "" },
        ],
      };
  const date = h("input", { type: "date", value: e.date });
  const desc = h("input", { value: e.description });
  const partner = h(
    "select",
    {},
    partnerOptions().map(([v, l]) => h("option", { value: v }, l)),
  );
  partner.value = e.partner ?? "";
  const body = h("tbody");
  const totals = h("div", {});
  const claimOptions = [["", "—"], ...S.book.claims.map((c) => [c.id, c.title])];
  const recalc = () => {
    let d = 0n,
      c = 0n,
      bad = false;
    for (const tr of body.children) {
      const dv = dec(tr.querySelector(".debit").value),
        cv = dec(tr.querySelector(".credit").value);
      if (dv === null || cv === null) bad = true;
      else {
        d += dv;
        c += cv;
      }
    }
    totals.className = !bad && d === c && d > 0n ? "balance-ok" : "balance-ng";
    totals.textContent = bad
      ? "金額を数値で入力してください"
      : `借方合計 ${fmt(decText(d))} ／ 貸方合計 ${fmt(decText(c))}${d === c ? (d > 0n ? " ✓ 一致" : "") : ` ／ 差額 ${fmt(decText(d - c))}`}`;
  };
  const addRow = (l) => {
    const claim = h(
      "select",
      { class: "claim" },
      claimOptions.map(([v, t]) => h("option", { value: v }, t)),
    );
    claim.value = l.claim ?? "";
    const tr = h(
      "tr",
      {},
      h(
        "td",
        {},
        accountSelect(() => true, l.account, "科目を選択"),
      ),
      h(
        "td",
        {},
        h("input", {
          class: "amount debit",
          inputmode: "decimal",
          value: isZero(l.debit) ? "" : l.debit,
          oninput: recalc,
        }),
      ),
      h(
        "td",
        {},
        h("input", {
          class: "amount credit",
          inputmode: "decimal",
          value: isZero(l.credit) ? "" : l.credit,
          oninput: recalc,
        }),
      ),
      h("td", {}, h("input", { class: "memo", value: l.memo || "" })),
      h("td", {}, claim),
      h(
        "td",
        {},
        h(
          "button",
          { type: "button", title: "行を削除", onclick: () => (tr.remove(), recalc()) },
          "×",
        ),
      ),
    );
    tr.querySelector("select").classList.add("account");
    body.append(tr);
  };
  e.lines.forEach(addRow);
  recalc();
  const voidBtn =
    entry && !S.readonly
      ? h(
          "button",
          {
            type: "button",
            class: "danger",
            onclick: async () => {
              try {
                await change(
                  { operation: "void_entry", id: entry.id, void: !entry.void },
                  entry.void ? "取消を解除しました" : "取り消しました",
                );
                m.close();
                render();
              } catch (err) {
                message(err.message, true);
              }
            },
          },
          entry.void ? "取消を解除" : "取り消す",
        )
      : null;
  const m = modal(
    entry ? `仕訳 No.${entry.id} を編集` : "仕訳を追加",
    h(
      "div",
      {},
      h(
        "div",
        { class: "form" },
        h("label", {}, "日付"),
        date,
        h("label", {}, "摘要"),
        desc,
        h("label", {}, "取引先"),
        partner,
      ),
      h("h3", {}, "明細（借方と貸方の合計を一致させます）"),
      h(
        "div",
        { class: "scroll" },
        h(
          "table",
          { class: "lines" },
          h(
            "thead",
            {},
            h(
              "tr",
              {},
              ["勘定科目", "借方", "貸方", "メモ", "債権債務", ""].map((t) => h("th", {}, t)),
            ),
          ),
          body,
        ),
      ),
      h(
        "div",
        { class: "toolbar", style: "margin-top:8px" },
        h(
          "button",
          { type: "button", onclick: () => addRow({ account: "", debit: "", credit: "" }) },
          "行を追加",
        ),
        totals,
      ),
      entry?.source
        ? h(
            "p",
            { class: "muted" },
            `自動計上（${sourceLabel(entry.source)}）の仕訳です。取消すと再計上できます。`,
          )
        : null,
    ),
    {
      wide: true,
      extra: voidBtn,
      onSubmit: async () => {
        const lines = [...body.children].map((tr) => ({
          account: Number(tr.querySelector(".account").value),
          debit: ascii(tr.querySelector(".debit").value) || "0",
          credit: ascii(tr.querySelector(".credit").value) || "0",
          memo: tr.querySelector(".memo").value,
          claim: idOrNull(tr.querySelector(".claim").value),
        }));
        if (lines.some((l) => !l.account)) throw new Error("すべての行で科目を選んでください");
        await change({
          operation: "save_entry",
          entry: {
            id: e.id,
            date: date.value,
            description: desc.value,
            partner: idOrNull(partner.value),
            lines,
          },
        });
        render();
      },
    },
  );
}

// ----- Accounts & partners -----
RENDER.accounts = async () => {
  const sub = S.sub.accounts || "list";
  const head = tabs("accounts", [
    ["list", "勘定科目"],
    ["guide", "使い分けガイド"],
    ["partners", "取引先"],
  ]);
  if (sub === "guide") {
    const q = h("input", {
      type: "search",
      placeholder: "例: ガソリン、10万円のパソコン、住民税",
      value: S.guideQuery || "",
    });
    const out = h("div");
    const run = async () => {
      S.guideQuery = q.value;
      const g = await api("account_guide", bookArgs({ query: q.value.trim() }));
      put(
        out,
        q.value.trim()
          ? h(
              "div",
              { class: "card" },
              h("h2", {}, "候補の科目"),
              g.suggestions.filter((x) => x.score > 0).length
                ? g.suggestions
                    .filter((x) => x.score > 0)
                    .slice(0, 6)
                    .map((x) =>
                      h(
                        "div",
                        { class: "guide", style: "margin-bottom:8px" },
                        h(
                          "h4",
                          {},
                          `${x.code} ${x.name}`,
                          h(
                            "span",
                            { class: "badge" },
                            `一致: ${x.matched.slice(0, 3).join("・")}`,
                          ),
                        ),
                        h("p", {}, x.guide),
                      ),
                    )
                : h("p", { class: "muted" }, "候補が見つかりません。"),
            )
          : null,
        h(
          "div",
          { class: "card" },
          h("h2", {}, q.value.trim() ? "関係する使い分け" : "迷いやすい科目の使い分け"),
          g.distinctions.map((d) => distinction(d)),
        ),
      );
    };
    q.addEventListener("input", () => {
      clearTimeout(S.guideTimer);
      S.guideTimer = setTimeout(run, 200);
    });
    await run();
    return h(
      "div",
      {},
      head,
      h("div", { class: "toolbar" }, h("label", { style: "flex:1" }, "何を記録しますか？", q)),
      out,
    );
  }
  if (sub === "partners") {
    return h(
      "div",
      {},
      head,
      h(
        "div",
        { class: "toolbar" },
        h(
          "button",
          { class: "primary", disabled: S.readonly, onclick: () => editPartner(null) },
          "取引先を追加",
        ),
      ),
      h(
        "div",
        { class: "card" },
        table(
          ["名称", "メモ", "状態"],
          S.book.partners.map((p) =>
            h(
              "tr",
              { class: "clickable", onclick: () => editPartner(p) },
              h("td", {}, p.name),
              h("td", {}, p.note),
              h("td", {}, p.archived ? "使用停止" : ""),
            ),
          ),
        ),
      ),
    );
  }
  const rows = [];
  for (const [cat, catLabel] of S.meta.categories) {
    rows.push(h("tr", { class: "group" }, h("td", { colspan: 5 }, catLabel)));
    for (const a of S.book.accounts
      .filter((a) => a.category === cat)
      .sort((x, y) => x.code.localeCompare(y.code)))
      rows.push(
        h(
          "tr",
          { class: "clickable" + (a.archived ? " void" : ""), onclick: () => editAccount(a) },
          h("td", {}, a.code),
          h(
            "td",
            {},
            a.name,
            a.payment ? h("span", { class: "badge good" }, "支払手段") : null,
            a.role ? h("span", { class: "badge" }, label("roles", a.role)) : null,
          ),
          h("td", { class: "muted" }, a.guide.length > 90 ? a.guide.slice(0, 90) + "…" : a.guide),
        ),
      );
  }
  return h(
    "div",
    {},
    head,
    h(
      "div",
      { class: "toolbar" },
      h(
        "button",
        { class: "primary", disabled: S.readonly, onclick: () => editAccount(null) },
        "勘定科目を追加",
      ),
      h("span", { class: "muted" }, "行をクリックすると使い分けの説明を確認・編集できます。"),
    ),
    h("div", { class: "card" }, table(["コード", "科目", "使い分けの説明"], rows)),
  );
};
function editAccount(a) {
  const f = fields([
    { key: "code", label: "コード", value: a?.code, required: true },
    { key: "name", label: "科目名", value: a?.name, required: true },
    {
      key: "category",
      label: "区分",
      type: "select",
      value: a?.category ?? "expense",
      options: S.meta.categories,
    },
    {
      key: "role",
      label: "役割",
      type: "select",
      value: a?.role ?? "",
      options: [["", "（なし）"], ...S.meta.roles],
      help: "自動計上（利息・事業主貸・減価償却など）に使う科目です。区分と一致させます。",
    },
    {
      key: "parent",
      label: "親科目",
      type: "select",
      value: a?.parent ?? "",
      options: accountOptions((x) => x.id !== a?.id, "（なし）"),
    },
    {
      key: "payment",
      label: "支払手段",
      type: "checkbox",
      value: a?.payment,
      help: "現金・預金・カード・電子マネーなど、かんたん入力の支払方法に表示します。",
    },
    { key: "guide", label: "使い分けの説明", type: "textarea", value: a?.guide, wide: true },
    {
      key: "keywords",
      label: "検索語（読点区切り）",
      value: (a?.keywords || []).join("、"),
      help: "科目を探すときの手掛かり（例: ガソリン、車検）",
    },
    {
      key: "archived",
      label: "使用停止",
      type: "checkbox",
      value: a?.archived,
      help: "過去の仕訳は残したまま、新しい入力で選べなくします。",
    },
  ]);
  modal(a ? `勘定科目「${a.name}」` : "勘定科目を追加", f.element, {
    extra:
      a && !S.readonly
        ? h(
            "button",
            { type: "button", class: "danger", onclick: () => remove("account", a.id) },
            "削除",
          )
        : null,
    onSubmit: async () => {
      const v = f.values();
      await change({
        operation: "save_account",
        account: {
          id: a?.id ?? 0,
          code: v.code,
          name: v.name,
          category: v.category,
          role: v.role || null,
          parent: idOrNull(v.parent),
          payment: v.payment,
          guide: v.guide,
          keywords: v.keywords
            .split(/[、,]/)
            .map((x) => x.trim())
            .filter(Boolean),
          archived: v.archived,
        },
      });
      render();
    },
  });
}
async function remove(target, id) {
  if (!confirm("削除しますか？使用中の場合は削除できません。")) return;
  try {
    await change({ operation: "remove", target, id }, "削除しました");
    document.querySelector(".modal-backdrop")?.remove();
    render();
  } catch (e) {
    message(e.message, true);
  }
}
function editPartner(p) {
  const f = fields([
    { key: "name", label: "名称", value: p?.name, required: true },
    { key: "note", label: "メモ", type: "textarea", value: p?.note },
    { key: "archived", label: "使用停止", type: "checkbox", value: p?.archived },
  ]);
  modal(p ? `取引先「${p.name}」` : "取引先を追加", f.element, {
    extra:
      p && !S.readonly
        ? h(
            "button",
            { type: "button", class: "danger", onclick: () => remove("partner", p.id) },
            "削除",
          )
        : null,
    onSubmit: async () => {
      await change({ operation: "save_partner", partner: { id: p?.id ?? 0, ...f.values() } });
      render();
    },
  });
}

// ----- Claims -----
RENDER.claims = async () => {
  const asOf = (S.claimDate ||= today());
  const r = await api("claim_report", bookArgs({ as_of: asOf, include_closed: !!S.claimClosed }));
  const bucketLabel = {
    current: "期日内",
    overdue_1_30: "1〜30日延滞",
    overdue_31_90: "31〜90日延滞",
    overdue_91_plus: "91日以上延滞",
  };
  const aging = (title, t) =>
    h(
      "div",
      { class: "card" },
      h("h2", {}, title),
      table(
        ["残高", "期日内", "1〜30日", "31〜90日", "91日以上"],
        [
          h(
            "tr",
            {},
            num(t.outstanding),
            num(t.current),
            num(t.overdue_1_30),
            num(t.overdue_31_90),
            num(t.overdue_91_plus),
          ),
        ],
        { numeric: [0, 1, 2, 3, 4] },
      ),
    );
  const rows = r.claims.map((c) => {
    const detail = h(
      "tr",
      { hidden: true },
      h(
        "td",
        { colspan: 9 },
        table(
          ["回", "期日", "元本", "利息", "支払額", "残高"],
          c.schedule.map((i) =>
            h(
              "tr",
              {},
              h("td", { class: "num" }, i.number),
              h("td", {}, i.date),
              num(i.principal),
              num(i.interest),
              num(i.payment),
              num(i.remaining),
            ),
          ),
          { numeric: [0, 2, 3, 4, 5] },
        ),
      ),
    );
    return [
      h(
        "tr",
        {},
        h(
          "td",
          {},
          c.title,
          c.closed ? h("span", { class: "badge good" }, "完了") : null,
          h("div", { class: "muted" }, c.partner || ""),
        ),
        h(
          "td",
          {},
          c.side === "receivable" ? "債権" : "債務",
          h("div", { class: "muted" }, c.account),
        ),
        num(c.principal),
        num(c.outstanding),
        num(c.arrears),
        h(
          "td",
          {},
          h(
            "span",
            { class: "badge " + (c.bucket === "current" ? "good" : "bad") },
            bucketLabel[c.bucket],
          ),
        ),
        h("td", {}, c.next ? `${c.next.date} ${fmt(c.next.payment)}` : "—"),
        h(
          "td",
          {},
          h("button", { onclick: () => (detail.hidden = !detail.hidden) }, "返済予定"),
          " ",
          c.closed
            ? null
            : h(
                "button",
                { disabled: S.readonly, onclick: () => settle(c) },
                c.side === "receivable" ? "回収" : "返済",
              ),
          " ",
          h(
            "button",
            { onclick: () => editClaim(S.book.claims.find((x) => x.id === c.id)) },
            "編集",
          ),
        ),
      ),
      detail,
    ];
  });
  return h(
    "div",
    {},
    h(
      "div",
      { class: "toolbar" },
      h(
        "label",
        {},
        "基準日",
        h("input", {
          type: "date",
          value: asOf,
          onchange: (e) => ((S.claimDate = e.target.value), render()),
        }),
      ),
      h(
        "label",
        {},
        h("input", {
          type: "checkbox",
          checked: !!S.claimClosed,
          onchange: (e) => ((S.claimClosed = e.target.checked), render()),
        }),
        "完了済みも表示",
      ),
      h("span", { class: "spacer" }),
      h(
        "button",
        { class: "primary", disabled: S.readonly, onclick: () => editClaim(null) },
        "債権・債務を登録",
      ),
    ),
    h(
      "div",
      { class: "grid2" },
      aging("債権（受け取る予定）", r.receivable),
      aging("債務（支払う予定）", r.payable),
    ),
    h(
      "div",
      { class: "card" },
      table(["件名", "区分", "元本", "残高", "延滞額", "状態", "次回", ""], rows, {
        numeric: [2, 3, 4],
      }),
    ),
  );
};
function editClaim(c) {
  const f = fields([
    {
      key: "side",
      label: "区分",
      type: "select",
      value: c?.side ?? "receivable",
      options: [
        ["receivable", "債権（貸した・売掛）"],
        ["payable", "債務（借りた・買掛・ローン）"],
      ],
    },
    { key: "title", label: "件名", value: c?.title, required: true },
    {
      key: "partner",
      label: "相手",
      type: "select",
      value: c?.partner ?? "",
      options: partnerOptions(),
    },
    {
      key: "account",
      label: "科目",
      type: "select",
      value: c?.account ?? "",
      options: accountOptions((a) => ["asset", "liability"].includes(a.category) && !a.payment),
      help: "債権は資産（売掛金・貸付金・立替金）、債務は負債（買掛金・未払金・借入金）",
    },
    { key: "principal", label: "元本", type: "decimal", value: c?.principal, required: true },
    { key: "issued", label: "発生日", type: "date", value: c?.issued ?? today() },
    { key: "due", label: "期日・初回返済日", type: "date", value: c?.due ?? "" },
    { key: "annual_rate", label: "年利（%）", type: "decimal", value: c?.annual_rate ?? "0" },
    {
      key: "repayment",
      label: "返済方法",
      type: "select",
      value: c?.repayment ?? "lump",
      options: S.meta.repayments,
      help: "元利均等は毎回の支払額が一定、元金均等は元本の返済額が一定です。",
    },
    { key: "installments", label: "回数（月）", type: "number", value: c?.installments ?? 0 },
    { key: "memo", label: "メモ", type: "textarea", value: c?.memo },
    ...(c
      ? [{ key: "closed", label: "完了", type: "checkbox", value: c.closed }]
      : [
          {
            key: "opening",
            label: "発生の仕訳を作る相手科目",
            type: "select",
            value: "",
            options: accountOptions(() => true, "（作らない）"),
            help: "例: 貸したお金→普通預金、ローンで車を購入→特別支出、売上の掛け→売上高",
          },
        ]),
  ]);
  modal(c ? `「${c.title}」を編集` : "債権・債務を登録", f.element, {
    extra:
      c && !S.readonly
        ? h(
            "button",
            { type: "button", class: "danger", onclick: () => remove("claim", c.id) },
            "削除",
          )
        : null,
    onSubmit: async () => {
      const v = f.values();
      const claim = {
        id: c?.id ?? 0,
        side: v.side,
        title: v.title,
        partner: idOrNull(v.partner),
        account: Number(v.account),
        principal: v.principal,
        issued: v.issued,
        due: v.due || null,
        annual_rate: v.annual_rate || "0",
        repayment: v.repayment,
        installments: v.installments || 0,
        closed: !!v.closed,
        memo: v.memo,
      };
      await change({ operation: "save_claim", claim, opening: c ? null : idOrNull(v.opening) });
      render();
    },
  });
}
function settle(c) {
  const next = c.next || c.schedule.find((i) => !isZero(i.principal));
  const f = fields([
    { key: "date", label: "日付", type: "date", value: today() },
    {
      key: "principal",
      label: "元本",
      type: "decimal",
      value: next
        ? dec(next.principal) > dec(c.outstanding)
          ? c.outstanding
          : next.principal
        : c.outstanding,
      help: `残高 ${fmt(c.outstanding)}`,
    },
    { key: "interest", label: "利息", type: "decimal", value: next?.interest ?? "0" },
    {
      key: "via",
      label: c.side === "receivable" ? "入金先" : "支払元",
      type: "select",
      value: "",
      options: accountOptions((a) => a.payment),
    },
  ]);
  modal(`${c.title}：${c.side === "receivable" ? "回収" : "返済"}`, f.element, {
    onSubmit: async () => {
      const v = f.values();
      await change({
        operation: "settle_claim",
        settlement: {
          claim: c.id,
          date: v.date,
          principal: v.principal,
          interest: v.interest || "0",
          via: Number(v.via),
        },
      });
      render();
    },
  });
}

// ----- Budget -----
RENDER.budget = async () => {
  const budgets = S.book.budgets;
  if (!budgets.find((b) => b.id === S.budgetId))
    S.budgetId = budgets.find((b) => b.fiscal_year === fyStart())?.id ?? budgets[0]?.id;
  const bar = h(
    "div",
    { class: "toolbar" },
    budgets.length
      ? h(
          "select",
          { onchange: (e) => ((S.budgetId = Number(e.target.value)), render()) },
          budgets.map((b) =>
            h(
              "option",
              { value: b.id, selected: b.id === S.budgetId },
              `${b.name}（${b.fiscal_year}年度）`,
            ),
          ),
        )
      : null,
    h("span", { class: "spacer" }),
    budgets.length
      ? h(
          "button",
          {
            disabled: S.readonly,
            onclick: () => editBudget(budgets.find((b) => b.id === S.budgetId)),
          },
          "予算を編集",
        )
      : null,
    h(
      "button",
      { class: "primary", disabled: S.readonly, onclick: () => editBudget(null) },
      "予算を作成",
    ),
  );
  if (!budgets.length)
    return h(
      "div",
      {},
      bar,
      h(
        "div",
        { class: "card" },
        h(
          "p",
          {},
          "予算はまだありません。年度ごとに科目別・月別の予算を作成すると実績と比較できます。",
        ),
      ),
    );
  const r = await api("budget_report", bookArgs({ budget_id: S.budgetId }));
  const row = (x) =>
    h(
      "tr",
      {},
      h("td", {}, x.name, x.unbudgeted ? h("span", { class: "badge warn" }, "予算なし") : null),
      num(x.total_budget),
      num(x.total_actual),
      num(
        x.total_variance,
        x.category === "expense" && !x.total_variance.startsWith("-") && !isZero(x.total_variance)
          ? "neg"
          : "",
      ),
      h("td", { class: "num" }, x.achievement ? `${x.achievement}%` : "—"),
      h("td", {}, meter(x.total_actual, x.total_budget)),
    );
  const monthly = (cat) =>
    table(
      ["科目", ...r.months.map((m) => m.slice(5) + "月")],
      r.rows
        .filter((x) => x.category === cat)
        .map((x) =>
          h(
            "tr",
            {},
            h("td", {}, x.name),
            x.budget.map((b, i) =>
              h(
                "td",
                {
                  class: "num",
                  title: `予算 ${fmt(b)} / 実績 ${fmt(x.actual[i])} / 差異 ${fmt(x.variance[i])}`,
                },
                h("div", {}, fmt(x.actual[i])),
                h("div", { class: "muted" }, fmt(b)),
              ),
            ),
          ),
        ),
      { numeric: r.months.map((_, i) => i + 1) },
    );
  const section = (cat, title, totals) =>
    h(
      "div",
      { class: "card" },
      h("h2", {}, title),
      table(
        ["科目", "年間予算", "実績", "差異（実績−予算）", "達成率", ""],
        [
          ...r.rows.filter((x) => x.category === cat).map(row),
          h(
            "tr",
            { class: "total" },
            h("td", {}, "合計"),
            num(totals.total_budget),
            num(totals.total_actual),
            num(decText(dec(totals.total_actual) - dec(totals.total_budget))),
            h("td"),
            h("td"),
          ),
        ],
        { numeric: [1, 2, 3, 4] },
      ),
      h("details", {}, h("summary", {}, "月別（上段: 実績、下段: 予算）"), monthly(cat)),
    );
  return h(
    "div",
    {},
    bar,
    h(
      "div",
      { class: "tiles" },
      tile("収入予算", r.income.total_budget, `実績 ${fmt(r.income.total_actual)}`),
      tile("支出予算", r.expense.total_budget, `実績 ${fmt(r.expense.total_actual)}`),
      tile("収支予算", r.net_budget, `実績 ${fmt(r.net_actual)}`),
    ),
    section("expense", "支出", r.expense),
    section("income", "収入", r.income),
  );
};
function editBudget(b) {
  const months = Array.from(
    { length: 12 },
    (_, i) => ((S.book.fiscal_start_month - 1 + i) % 12) + 1,
  );
  const name = h("input", { value: b?.name ?? `${fyStart()}年度 予算`, required: true });
  const year = h("input", { type: "number", value: b?.fiscal_year ?? fyStart() });
  const memo = h("input", { value: b?.memo ?? "" });
  const body = h("tbody");
  const used = new Map((b?.lines || []).map((l) => [l.account, l.amounts]));
  for (const a of S.book.accounts
    .filter((a) => ["income", "expense"].includes(a.category) && (!a.archived || used.has(a.id)))
    .sort((x, y) => (x.category + x.code).localeCompare(y.category + y.code))) {
    const amounts = used.get(a.id) || Array(12).fill("");
    const inputs = amounts.map((v) =>
      h("input", {
        class: "amount",
        inputmode: "decimal",
        value: isZero(v) && !used.has(a.id) ? "" : v,
        style: "width:84px",
      }),
    );
    const all = h("input", {
      class: "amount",
      inputmode: "decimal",
      placeholder: "全月",
      style: "width:84px",
      onchange: (e) => inputs.forEach((i) => (i.value = e.target.value)),
    });
    const tr = h(
      "tr",
      { "data-account": a.id },
      h(
        "td",
        {},
        `${a.code} ${a.name}`,
        h("div", { class: "muted" }, label("categories", a.category)),
      ),
      h("td", {}, all),
      inputs.map((i) => h("td", {}, i)),
    );
    body.append(tr);
  }
  modal(
    b ? `予算「${b.name}」を編集` : "予算を作成",
    h(
      "div",
      {},
      h(
        "div",
        { class: "form" },
        h("label", {}, "名称"),
        name,
        h("label", {}, "年度（開始年）"),
        year,
        h("label", {}, "メモ"),
        memo,
      ),
      h(
        "p",
        { class: "muted" },
        "空欄の科目は予算に含めません。「全月」に入力すると12か月に同じ額を入れます。",
      ),
      h(
        "div",
        { class: "scroll" },
        h(
          "table",
          { class: "lines" },
          h(
            "thead",
            {},
            h(
              "tr",
              {},
              h("th", {}, "科目"),
              h("th", {}, "全月"),
              months.map((m) => h("th", { class: "num" }, `${m}月`)),
            ),
          ),
          body,
        ),
      ),
    ),
    {
      wide: true,
      extra:
        b && !S.readonly
          ? h(
              "button",
              { type: "button", class: "danger", onclick: () => remove("budget", b.id) },
              "削除",
            )
          : null,
      onSubmit: async () => {
        const lines = [];
        for (const tr of body.children) {
          const values = [...tr.querySelectorAll("td:nth-child(n+3) input")].map((i) =>
            ascii(i.value),
          );
          if (values.every((v) => v === "")) continue;
          lines.push({ account: Number(tr.dataset.account), amounts: values.map((v) => v || "0") });
        }
        const r = await change({
          operation: "save_budget",
          budget: {
            id: b?.id ?? 0,
            name: name.value,
            fiscal_year: Number(year.value),
            memo: memo.value,
            lines,
          },
        });
        S.budgetId = r.id;
        render();
      },
    },
  );
}

// ----- Plans -----
RENDER.plans = async () => {
  const t = today();
  const from = addMonths(t.slice(0, 8) + "01", -12);
  const to = monthEnd(addMonths(t.slice(0, 8) + "01", 3));
  const cal = await api("plan_calendar", bookArgs({ from, to }));
  const pending = cal.occurrences.filter((o) => !o.posted && o.date <= t);
  const upcoming = cal.occurrences.filter((o) => o.date > t);
  const next = (p) =>
    cal.occurrences.find((o) => o.plan === p.id && !o.posted && o.date >= t)?.date ?? "—";
  return h(
    "div",
    {},
    h(
      "div",
      { class: "toolbar" },
      h(
        "span",
        { class: "muted" },
        "家賃・保険料などの定期的な取引や、予定している大きな支出を登録します。計上するまで帳簿には影響しません。予測では未計上の予定として使われます。",
      ),
      h("span", { class: "spacer" }),
      h(
        "button",
        { class: "primary", disabled: S.readonly, onclick: () => editPlan(null) },
        "計画を追加",
      ),
    ),
    h(
      "div",
      { class: "card" },
      h("h2", {}, "計画"),
      table(
        ["名称", "頻度", "金額", "借方 ／ 貸方", "次回", "状態"],
        S.book.plans.map((p) =>
          h(
            "tr",
            { class: "clickable", onclick: () => editPlan(p) },
            h("td", {}, p.name),
            h(
              "td",
              {},
              p.frequency === "once"
                ? "1回"
                : `${p.interval > 1 ? p.interval : ""}${label("frequencies", p.frequency)}`.replace(
                    /^(\d+)毎/,
                    "$1",
                  ),
            ),
            num(p.amount),
            h("td", {}, `${accountName(p.debit)} ／ ${accountName(p.credit)}`),
            h("td", {}, next(p)),
            h("td", {}, p.active ? "有効" : h("span", { class: "badge" }, "停止")),
          ),
        ),
        { numeric: [2] },
      ),
    ),
    h(
      "div",
      { class: "grid2" },
      h(
        "div",
        { class: "card" },
        h("h2", {}, `計上待ち（${pending.length}件）`),
        pending.length
          ? table(
              ["予定日", "計画", "金額", ""],
              pending.map((o) =>
                h(
                  "tr",
                  {},
                  h("td", {}, o.date),
                  h("td", {}, o.name),
                  num(o.amount),
                  h("td", {}, postButton(o)),
                ),
              ),
              { numeric: [2] },
            )
          : h("p", { class: "muted" }, "ありません。"),
      ),
      h(
        "div",
        { class: "card" },
        h("h2", {}, "今後3か月の予定"),
        table(
          ["予定日", "計画", "金額"],
          upcoming.map((o) => h("tr", {}, h("td", {}, o.date), h("td", {}, o.name), num(o.amount))),
          { numeric: [2] },
        ),
      ),
    ),
  );
};
function editPlan(p) {
  const f = fields([
    { key: "name", label: "名称", value: p?.name, required: true },
    {
      key: "debit",
      label: "借方（費用・増える側）",
      type: "select",
      value: p?.debit ?? "",
      options: accountOptions(),
    },
    {
      key: "credit",
      label: "貸方（支払元・減る側）",
      type: "select",
      value: p?.credit ?? "",
      options: accountOptions(),
    },
    { key: "amount", label: "金額", type: "decimal", value: p?.amount, required: true },
    { key: "start", label: "開始日（初回）", type: "date", value: p?.start ?? today() },
    { key: "end", label: "終了日", type: "date", value: p?.end ?? "" },
    {
      key: "frequency",
      label: "頻度",
      type: "select",
      value: p?.frequency ?? "monthly",
      options: S.meta.frequencies,
    },
    {
      key: "interval",
      label: "間隔",
      type: "number",
      value: p?.interval ?? 1,
      help: "例: 頻度「毎月」・間隔2 で2か月ごと",
    },
    {
      key: "partner",
      label: "取引先",
      type: "select",
      value: p?.partner ?? "",
      options: partnerOptions(),
    },
    { key: "active", label: "有効", type: "checkbox", value: p ? p.active : true },
    { key: "memo", label: "メモ", type: "textarea", value: p?.memo },
  ]);
  modal(p ? `計画「${p.name}」` : "計画を追加", f.element, {
    extra:
      p && !S.readonly
        ? h(
            "button",
            { type: "button", class: "danger", onclick: () => remove("plan", p.id) },
            "削除",
          )
        : null,
    onSubmit: async () => {
      const v = f.values();
      await change({
        operation: "save_plan",
        plan: {
          id: p?.id ?? 0,
          name: v.name,
          debit: Number(v.debit),
          credit: Number(v.credit),
          amount: v.amount,
          start: v.start,
          end: v.end || null,
          frequency: v.frequency,
          interval: v.interval || 1,
          partner: idOrNull(v.partner),
          active: v.active,
          memo: v.memo,
        },
      });
      render();
    },
  });
}

// ----- Cost management -----
RENDER.cost = async () => {
  const asOf = (S.costDate ||= today());
  const r = await api("cost_report", bookArgs({ as_of: asOf }));
  const sub = S.sub.cost || "products";
  const head = h(
    "div",
    {},
    tabs("cost", [
      ["products", "製品原価"],
      ["productions", "製造実績"],
      ["materials", "原材料"],
      ["equipment", "設備"],
    ]),
    h(
      "div",
      { class: "toolbar" },
      h(
        "label",
        {},
        "基準日",
        h("input", {
          type: "date",
          value: asOf,
          onchange: (e) => ((S.costDate = e.target.value), render()),
        }),
      ),
    ),
  );
  const add = (text, fn) =>
    h("button", { class: "primary", disabled: S.readonly, onclick: fn }, text);
  if (sub === "materials")
    return h(
      "div",
      {},
      head,
      h(
        "div",
        { class: "toolbar" },
        add("原材料を追加", () => editMaterial(null)),
        h(
          "button",
          { disabled: S.readonly || !S.book.materials.length, onclick: () => editMove(null) },
          "仕入・棚卸調整を記録",
        ),
      ),
      h(
        "div",
        { class: "card" },
        h("h2", {}, `在庫（移動平均法） 合計 ${fmt(r.inventory_value)}`),
        table(
          ["原材料", "在庫数量", "平均単価", "在庫金額", "標準単価", ""],
          r.stock.map((x) =>
            h(
              "tr",
              {},
              h(
                "td",
                {},
                x.name,
                x.warnings.length
                  ? h("span", { class: "badge bad", title: x.warnings.join("\n") }, "在庫不足")
                  : null,
              ),
              h("td", { class: "num" }, `${fmt(x.quantity)} ${x.unit}`),
              num(x.average_cost),
              num(x.value),
              num(x.standard_cost),
              h(
                "td",
                {},
                h(
                  "button",
                  {
                    onclick: () => editMaterial(S.book.materials.find((m) => m.id === x.material)),
                  },
                  "編集",
                ),
              ),
            ),
          ),
          { numeric: [1, 2, 3, 4] },
        ),
      ),
      h(
        "div",
        { class: "card" },
        h("h2", {}, "仕入・棚卸調整（新しい順）"),
        table(
          ["日付", "原材料", "種類", "数量", "単価", ""],
          [...S.book.material_moves]
            .reverse()
            .slice(0, 100)
            .map((m) =>
              h(
                "tr",
                {},
                h("td", {}, m.date),
                h("td", {}, S.book.materials.find((x) => x.id === m.material)?.name),
                h("td", {}, m.kind === "purchase" ? "仕入" : "棚卸調整"),
                num(m.quantity),
                num(m.unit_cost ?? ""),
                h("td", {}, h("button", { onclick: () => editMove(m) }, "編集")),
              ),
            ),
          { numeric: [3, 4] },
        ),
      ),
    );
  if (sub === "equipment")
    return h(
      "div",
      {},
      head,
      h(
        "div",
        { class: "toolbar" },
        add("設備を追加", () => editEquipment(null)),
        h(
          "span",
          { class: "muted" },
          "時間当たり費用 ＝ (取得価額−残存価額) ÷ 耐用年数 ÷ 年間稼働時間 ＋ 運転費。減価償却は定額法・月割です。",
        ),
      ),
      ...r.equipment.map((e) =>
        h(
          "div",
          { class: "card" },
          h(
            "h2",
            {},
            e.name,
            " ",
            h(
              "button",
              { onclick: () => editEquipment(S.book.equipment.find((x) => x.id === e.equipment)) },
              "編集",
            ),
          ),
          h(
            "div",
            { class: "tiles" },
            tile("時間当たり費用", e.hourly_rate),
            tile("年間償却額", e.annual_depreciation),
            tile("帳簿価額（基準日）", e.book_value),
          ),
          table(
            ["年度", "償却額", "累計額", "期末帳簿価額", ""],
            e.schedule.map((x) =>
              h(
                "tr",
                {},
                h("td", {}, `${x.fiscal_year}年度`),
                num(x.amount),
                num(x.accumulated),
                num(x.book_value),
                h(
                  "td",
                  {},
                  x.posted
                    ? h("span", { class: "badge good" }, "計上済み")
                    : h(
                        "button",
                        { disabled: S.readonly, onclick: () => postDepreciation(e, x) },
                        "仕訳を計上",
                      ),
                ),
              ),
            ),
            { numeric: [1, 2, 3] },
          ),
        ),
      ),
    );
  if (sub === "productions")
    return h(
      "div",
      {},
      head,
      h(
        "div",
        { class: "toolbar" },
        add("製造実績を記録", () => editProduction(null)),
        h(
          "span",
          { class: "muted" },
          "材料の消費量・稼働時間・作業時間を空欄にすると標準（製品の構成）×数量で計算します。",
        ),
      ),
      h(
        "div",
        { class: "card" },
        table(
          [
            "日付",
            "製品",
            "数量",
            "実際原価",
            "単位原価",
            "標準原価",
            "差異",
            "材料差異",
            "設備差異",
            "労務差異",
            "",
          ],
          [...r.productions].reverse().map((p) =>
            h(
              "tr",
              {},
              h("td", {}, p.date),
              h("td", {}, p.name),
              num(p.quantity),
              num(p.actual.total),
              num(p.actual_unit_cost),
              num(p.standard_total),
              num(p.variance),
              num(p.variance_materials),
              num(p.variance_equipment),
              num(p.variance_labor),
              h(
                "td",
                {},
                h(
                  "button",
                  {
                    onclick: () =>
                      editProduction(S.book.productions.find((x) => x.id === p.production)),
                  },
                  "編集",
                ),
              ),
            ),
          ),
          { numeric: [2, 3, 4, 5, 6, 7, 8, 9] },
        ),
      ),
    );
  return h(
    "div",
    {},
    head,
    h(
      "div",
      { class: "toolbar" },
      add("製品を追加", () => editProduct(null)),
      h(
        "span",
        { class: "muted" },
        "標準原価 ＝ 材料（ロス込み使用量×標準単価）＋ 設備（時間×時間当たり費用）＋ 労務（時間×賃率）＋ 製造間接費（直接費×配賦率）",
      ),
    ),
    h(
      "div",
      { class: "grid2" },
      r.products.map((p) =>
        h(
          "div",
          { class: "card" },
          h(
            "h2",
            {},
            `${p.name}（1${p.unit}あたり）`,
            " ",
            h(
              "button",
              { onclick: () => editProduct(S.book.products.find((x) => x.id === p.product)) },
              "編集",
            ),
          ),
          table(
            ["費目", "数量", "単価", "金額"],
            [
              ...p.standard.lines.map((l) =>
                h(
                  "tr",
                  {},
                  h("td", {}, l.kind === "material" ? "材料：" : "設備：", l.name),
                  num(l.quantity),
                  num(l.unit_cost),
                  num(l.amount),
                ),
              ),
              h("tr", {}, h("td", {}, "労務費"), h("td"), h("td"), num(p.standard.labor)),
              h("tr", {}, h("td", {}, "製造間接費"), h("td"), h("td"), num(p.standard.overhead)),
              h(
                "tr",
                { class: "total" },
                h("td", {}, "標準原価"),
                h("td"),
                h("td"),
                num(p.standard.total),
              ),
            ],
            { numeric: [1, 2, 3] },
          ),
        ),
      ),
    ),
  );
};
function editMaterial(m) {
  const f = fields([
    { key: "name", label: "名称", value: m?.name, required: true },
    { key: "unit", label: "単位", value: m?.unit ?? "kg", required: true },
    {
      key: "standard_cost",
      label: "標準単価",
      type: "decimal",
      value: m?.standard_cost ?? "0",
      help: "0 のときは移動平均単価で標準原価を計算します",
    },
    { key: "archived", label: "使用停止", type: "checkbox", value: m?.archived },
    { key: "memo", label: "メモ", type: "textarea", value: m?.memo },
  ]);
  modal(m ? `原材料「${m.name}」` : "原材料を追加", f.element, {
    extra:
      m && !S.readonly
        ? h(
            "button",
            { type: "button", class: "danger", onclick: () => remove("material", m.id) },
            "削除",
          )
        : null,
    onSubmit: async () => {
      const v = f.values();
      await change({
        operation: "save_material",
        material: { id: m?.id ?? 0, ...v, standard_cost: v.standard_cost || "0" },
      });
      render();
    },
  });
}
function editMove(m) {
  const f = fields([
    {
      key: "material",
      label: "原材料",
      type: "select",
      value: m?.material ?? S.book.materials[0]?.id,
      options: S.book.materials.map((x) => [x.id, `${x.name}（${x.unit}）`]),
    },
    { key: "date", label: "日付", type: "date", value: m?.date ?? today() },
    {
      key: "kind",
      label: "種類",
      type: "select",
      value: m?.kind ?? "purchase",
      options: [
        ["purchase", "仕入（受入）"],
        ["adjust", "棚卸調整（減少はマイナス）"],
      ],
    },
    { key: "quantity", label: "数量", type: "decimal", value: m?.quantity, required: true },
    {
      key: "unit_cost",
      label: "単価",
      type: "decimal",
      value: m?.unit_cost ?? "",
      help: "仕入は必須。棚卸調整の増加で空欄なら移動平均単価",
    },
    { key: "memo", label: "メモ", value: m?.memo },
  ]);
  modal(
    m ? "入出庫を編集" : "仕入・棚卸調整を記録",
    h(
      "div",
      {},
      f.element,
      h("p", { class: "muted" }, "仕入代金の支払いは仕訳（原材料／買掛金 など）で別に記録します。"),
    ),
    {
      extra:
        m && !S.readonly
          ? h(
              "button",
              { type: "button", class: "danger", onclick: () => remove("material_move", m.id) },
              "削除",
            )
          : null,
      onSubmit: async () => {
        const v = f.values();
        await change({
          operation: "save_material_move",
          material_move: {
            id: m?.id ?? 0,
            material: Number(v.material),
            date: v.date,
            kind: v.kind,
            quantity: v.quantity,
            unit_cost: v.unit_cost,
            memo: v.memo,
          },
        });
        render();
      },
    },
  );
}
function editEquipment(e) {
  const f = fields([
    { key: "name", label: "名称", value: e?.name, required: true },
    { key: "acquired", label: "取得日（事業供用日）", type: "date", value: e?.acquired ?? today() },
    { key: "cost", label: "取得価額", type: "decimal", value: e?.cost, required: true },
    {
      key: "salvage",
      label: "残存価額",
      type: "decimal",
      value: e?.salvage ?? "0",
      help: "現行の税法では通常0（備忘価額1円まで償却）",
    },
    { key: "life_years", label: "耐用年数", type: "number", value: e?.life_years ?? 5 },
    {
      key: "annual_hours",
      label: "年間稼働時間",
      type: "decimal",
      value: e?.annual_hours ?? "1000",
    },
    {
      key: "running_cost_per_hour",
      label: "運転費（1時間）",
      type: "decimal",
      value: e?.running_cost_per_hour ?? "0",
      help: "電気代・保守費など",
    },
    { key: "archived", label: "使用停止", type: "checkbox", value: e?.archived },
    { key: "memo", label: "メモ", type: "textarea", value: e?.memo },
  ]);
  modal(e ? `設備「${e.name}」` : "設備を追加", f.element, {
    extra:
      e && !S.readonly
        ? h(
            "button",
            { type: "button", class: "danger", onclick: () => remove("equipment", e.id) },
            "削除",
          )
        : null,
    onSubmit: async () => {
      const v = f.values();
      await change({
        operation: "save_equipment",
        equipment: {
          id: e?.id ?? 0,
          ...v,
          salvage: v.salvage || "0",
          running_cost_per_hour: v.running_cost_per_hour || "0",
        },
      });
      render();
    },
  });
}
function postDepreciation(e, row) {
  const [y] = [row.fiscal_year];
  const endMonth = ((S.book.fiscal_start_month + 10) % 12) + 1;
  const endYear = S.book.fiscal_start_month === 1 ? y : y + 1;
  const f = fields([
    {
      key: "date",
      label: "計上日",
      type: "date",
      value: monthEnd(`${endYear}-${String(endMonth).padStart(2, "0")}-01`),
    },
  ]);
  modal(
    `${e.name}：${y}年度の減価償却 ${fmt(row.amount)}`,
    h("div", {}, h("p", {}, "減価償却費 ／ 減価償却累計額 で計上します。"), f.element),
    {
      submit: "計上",
      onSubmit: async () => {
        await change(
          {
            operation: "post_depreciation",
            equipment: e.equipment,
            fiscal_year: y,
            date: f.values().date,
          },
          "計上しました",
        );
        render();
      },
    },
  );
}
function itemRows(items, options, key, cols) {
  const body = h("tbody");
  const add = (it = {}) => {
    const sel = h(
      "select",
      {},
      options.map(([v, l]) => h("option", { value: v }, l)),
    );
    if (it[key]) sel.value = it[key];
    const inputs = cols.map(([k]) =>
      h("input", { class: "amount", inputmode: "decimal", "data-key": k, value: it[k] ?? "" }),
    );
    const tr = h(
      "tr",
      {},
      h("td", {}, sel),
      inputs.map((i) => h("td", {}, i)),
      h("td", {}, h("button", { type: "button", onclick: () => tr.remove() }, "×")),
    );
    body.append(tr);
  };
  items.forEach(add);
  return {
    element: h(
      "div",
      {},
      h(
        "table",
        { class: "lines" },
        h(
          "thead",
          {},
          h(
            "tr",
            {},
            h("th", {}, key === "material" ? "原材料" : "設備"),
            cols.map(([, l]) => h("th", {}, l)),
            h("th"),
          ),
        ),
        body,
      ),
      h("button", { type: "button", onclick: () => add() }, "行を追加"),
    ),
    values: () =>
      [...body.children].map((tr) => {
        const o = { [key]: Number(tr.querySelector("select").value) };
        for (const i of tr.querySelectorAll("input"))
          if (i.value.trim() !== "") o[i.dataset.key] = ascii(i.value);
        for (const [k] of cols) o[k] ??= "0";
        return o;
      }),
  };
}
function editProduct(p) {
  const f = fields([
    { key: "name", label: "名称", value: p?.name, required: true },
    { key: "unit", label: "単位", value: p?.unit ?? "個", required: true },
    {
      key: "labor_hours",
      label: "標準作業時間（1単位）",
      type: "decimal",
      value: p?.labor_hours ?? "0",
    },
    {
      key: "labor_rate",
      label: "賃率（1時間）",
      type: "decimal",
      value: p?.labor_rate ?? "0",
      help: "自分の作業も時給換算すると価格設定の目安になります",
    },
    {
      key: "overhead_percent",
      label: "製造間接費配賦率（%）",
      type: "decimal",
      value: p?.overhead_percent ?? "0",
    },
    { key: "archived", label: "使用停止", type: "checkbox", value: p?.archived },
    { key: "memo", label: "メモ", type: "textarea", value: p?.memo },
  ]);
  const mats = itemRows(
    p?.materials || [],
    S.book.materials.map((m) => [m.id, `${m.name}（${m.unit}）`]),
    "material",
    [
      ["quantity", "使用量"],
      ["loss_percent", "ロス率%"],
    ],
  );
  const eqs = itemRows(
    p?.equipment || [],
    S.book.equipment.map((e) => [e.id, e.name]),
    "equipment",
    [["hours", "稼働時間"]],
  );
  modal(
    p ? `製品「${p.name}」` : "製品を追加",
    h(
      "div",
      {},
      f.element,
      h("h3", {}, "原材料（1単位あたり）"),
      mats.element,
      h("h3", {}, "設備（1単位あたり）"),
      eqs.element,
    ),
    {
      wide: true,
      extra:
        p && !S.readonly
          ? h(
              "button",
              { type: "button", class: "danger", onclick: () => remove("product", p.id) },
              "削除",
            )
          : null,
      onSubmit: async () => {
        const v = f.values();
        await change({
          operation: "save_product",
          product: {
            id: p?.id ?? 0,
            ...v,
            labor_hours: v.labor_hours || "0",
            labor_rate: v.labor_rate || "0",
            overhead_percent: v.overhead_percent || "0",
            materials: mats.values(),
            equipment: eqs.values(),
          },
        });
        render();
      },
    },
  );
}
function editProduction(p) {
  const f = fields([
    {
      key: "product",
      label: "製品",
      type: "select",
      value: p?.product ?? S.book.products[0]?.id,
      options: S.book.products.map((x) => [x.id, x.name]),
    },
    { key: "date", label: "製造日", type: "date", value: p?.date ?? today() },
    { key: "quantity", label: "数量", type: "decimal", value: p?.quantity, required: true },
    {
      key: "labor_hours",
      label: "作業時間（合計）",
      type: "decimal",
      value: p && !isZero(p.labor_hours) ? p.labor_hours : "",
      help: "空欄は標準作業時間×数量",
    },
    { key: "memo", label: "メモ", value: p?.memo },
  ]);
  const mats = itemRows(
    p?.materials || [],
    S.book.materials.map((m) => [m.id, `${m.name}（${m.unit}）`]),
    "material",
    [["quantity", "実際消費量"]],
  );
  const eqs = itemRows(
    p?.equipment || [],
    S.book.equipment.map((e) => [e.id, e.name]),
    "equipment",
    [["hours", "実際稼働時間"]],
  );
  modal(
    p ? "製造実績を編集" : "製造実績を記録",
    h(
      "div",
      {},
      f.element,
      h("h3", {}, "実際の材料消費（空なら標準）"),
      mats.element,
      h("h3", {}, "実際の設備稼働（空なら標準）"),
      eqs.element,
    ),
    {
      wide: true,
      extra:
        p && !S.readonly
          ? h(
              "button",
              { type: "button", class: "danger", onclick: () => remove("production", p.id) },
              "削除",
            )
          : null,
      onSubmit: async () => {
        const v = f.values();
        await change({
          operation: "save_production",
          production: {
            id: p?.id ?? 0,
            product: Number(v.product),
            date: v.date,
            quantity: v.quantity,
            labor_hours: v.labor_hours || "0",
            memo: v.memo,
            materials: mats.values(),
            equipment: eqs.values(),
          },
        });
        render();
      },
    },
  );
}

// ----- Statistics & forecast -----
RENDER.stats = async () => {
  const t = today();
  const o = (S.statsOptions ||= {
    from: addMonths(t.slice(0, 8) + "01", -12),
    to: monthEnd(addMonths(t.slice(0, 8) + "01", -1)),
    months: 6,
    history: 24,
    method: "auto",
  });
  const [st, fc, hist] = await Promise.all([
    api("statistics", bookArgs({ from: o.from, to: o.to })),
    api(
      "forecast",
      bookArgs({ as_of: t, months: o.months, history_months: o.history, method: o.method }),
    ),
    api(
      "statistics",
      bookArgs({
        from: addMonths(t.slice(0, 8) + "01", -12),
        to: monthEnd(addMonths(t.slice(0, 8) + "01", -1)),
      }),
    ),
  ]);
  const set =
    (k, conv = (x) => x) =>
    (e) => ((o[k] = conv(e.target.value)), render());
  const presets = [
    ["直近12か月", -12],
    ["直近6か月", -6],
    ["直近24か月", -24],
  ].map(([l, n]) =>
    h(
      "button",
      {
        onclick: () => (
          (o.from = addMonths(t.slice(0, 8) + "01", n)),
          (o.to = monthEnd(addMonths(t.slice(0, 8) + "01", -1))),
          render()
        ),
      },
      l,
    ),
  );
  const accRow = (a) =>
    h(
      "tr",
      {},
      h("td", {}, a.name),
      num(a.total),
      h("td", { class: "num" }, `${a.share}%`),
      num(a.mean),
      num(a.median),
      num(a.stdev),
      num(a.min),
      num(a.max),
      h(
        "td",
        { class: "num" },
        a.variability
          ? h(
              "span",
              { class: "badge " + (Number(a.variability) > 60 ? "warn" : "") },
              `${a.variability}%`,
            )
          : "—",
      ),
      h("td", { class: "num" }, `${a.active_months}/${st.months.length}`),
    );
  const fcRows = (cat) =>
    fc.accounts
      .filter((a) => a.category === cat)
      .map((a) =>
        h(
          "tr",
          {},
          h(
            "td",
            {},
            a.name,
            h(
              "div",
              { class: "muted" },
              `${label("methods", a.method)}${a.error ? ` · 誤差 ${fmt(a.error)}` : ""}`,
            ),
          ),
          num(a.mean),
          h(
            "td",
            { class: "num" + (a.trend.startsWith("-") ? " neg" : "") },
            (a.trend.startsWith("-") ? "" : "+") + fmt(a.trend),
          ),
          a.months.map((m) =>
            h(
              "td",
              {
                class: "num",
                title: `統計 ${fmt(m.baseline)} ＋ 予定 ${fmt(m.scheduled)}\n範囲 ${fmt(m.low)}〜${fmt(m.high)}`,
              },
              h("div", {}, fmt(m.total)),
              !isZero(m.scheduled)
                ? h("div", { class: "muted" }, `予定 ${fmt(m.scheduled)}`)
                : null,
            ),
          ),
        ),
      );
  return h(
    "div",
    {},
    h(
      "div",
      { class: "toolbar" },
      presets,
      h(
        "label",
        {},
        "期間",
        h("input", { type: "date", value: o.from, onchange: set("from") }),
        "〜",
        h("input", { type: "date", value: o.to, onchange: set("to") }),
      ),
    ),
    h(
      "div",
      { class: "tiles" },
      tile("期間の収入", st.total_income, `月平均 ${fmt(st.average_income)}`),
      tile("期間の支出", st.total_expense, `月平均 ${fmt(st.average_expense)}`),
      tile("貯蓄率（収支÷収入）", st.savings_rate ? `${st.savings_rate}%` : "—"),
    ),
    h("div", { class: "card" }, h("h2", {}, "月別の収入と支出"), barChart(st.months)),
    h(
      "div",
      { class: "card" },
      h("h2", {}, "支出の科目別統計"),
      h(
        "p",
        { class: "muted" },
        "変動係数（標準偏差÷平均）が大きい科目は月ごとのばらつきが大きく、予測の幅も広がります。",
      ),
      table(
        [
          "科目",
          "合計",
          "構成比",
          "月平均",
          "中央値",
          "標準偏差",
          "最小",
          "最大",
          "変動係数",
          "発生月",
        ],
        st.accounts.filter((a) => a.category === "expense").map(accRow),
        { numeric: [1, 2, 3, 4, 5, 6, 7, 8, 9] },
      ),
      h(
        "details",
        {},
        h("summary", {}, "収入の科目別統計"),
        table(
          [
            "科目",
            "合計",
            "構成比",
            "月平均",
            "中央値",
            "標準偏差",
            "最小",
            "最大",
            "変動係数",
            "発生月",
          ],
          st.accounts.filter((a) => a.category === "income").map(accRow),
          { numeric: [1, 2, 3, 4, 5, 6, 7, 8, 9] },
        ),
      ),
    ),
    h(
      "div",
      { class: "card" },
      h("h2", {}, "今後の支出予想"),
      h(
        "div",
        { class: "toolbar" },
        h(
          "label",
          {},
          "予測",
          h(
            "select",
            { onchange: set("months", Number) },
            [3, 6, 12, 24].map((n) =>
              h("option", { value: n, selected: n === o.months }, `${n}か月`),
            ),
          ),
        ),
        h(
          "label",
          {},
          "履歴",
          h(
            "select",
            { onchange: set("history", Number) },
            [6, 12, 24, 36, 60].map((n) =>
              h("option", { value: n, selected: n === o.history }, `${n}か月`),
            ),
          ),
        ),
        h(
          "label",
          {},
          "方法",
          h(
            "select",
            { onchange: set("method") },
            S.meta.methods.map(([k, l]) => h("option", { value: k, selected: k === o.method }, l)),
          ),
        ),
      ),
      h(
        "p",
        { class: "muted" },
        "統計（自動選択では移動平均・線形トレンド・季節変動のうち直近の検証誤差が最小の方法）による予測に、未計上の計画と債権債務の返済予定を加えています。計画から計上した過去の取引は二重計上を避けるため統計から除きます。今月は途中のため来月から予測します。",
      ),
      fc.notes.length
        ? h(
            "ul",
            {},
            fc.notes.map((n) => h("li", { class: "muted" }, n)),
          )
        : null,
      forecastChart(hist.months, fc),
      table(
        ["月", "収入予想", "支出予想", "支出の幅", "収支", "予定の返済・振替", "予定の入金"],
        fc.totals.map((x) =>
          h(
            "tr",
            {},
            h("td", {}, x.month),
            num(x.income),
            num(x.expense),
            h("td", { class: "num" }, `${fmt(x.expense_low)}〜${fmt(x.expense_high)}`),
            num(x.net),
            num(x.planned_payments),
            num(x.planned_receipts),
          ),
        ),
        { numeric: [1, 2, 3, 4, 5, 6] },
      ),
      h("h3", {}, "科目別の支出予想"),
      table(
        ["科目", "月平均", "傾向/月", ...fc.months.map((m) => m.slice(2).replace("-", "/"))],
        fcRows("expense"),
        { numeric: [1, 2, ...fc.months.map((_, i) => i + 3)] },
      ),
      h(
        "details",
        {},
        h("summary", {}, "科目別の収入予想"),
        table(
          ["科目", "月平均", "傾向/月", ...fc.months.map((m) => m.slice(2).replace("-", "/"))],
          fcRows("income"),
          { numeric: [1, 2, ...fc.months.map((_, i) => i + 3)] },
        ),
      ),
    ),
  );
};

// ----- Reports -----
RENDER.reports = async () => {
  const sub = S.sub.reports || "pl";
  const t = today();
  const o = (S.reportOptions ||= {
    from: `${fyStart()}-${String(S.book.fiscal_start_month).padStart(2, "0")}-01`,
    to: t,
    account: S.book.accounts.find((a) => a.role === "bank")?.id,
  });
  const set =
    (k, conv = (x) => x) =>
    (e) => ((o[k] = conv(e.target.value)), render());
  const head = h(
    "div",
    {},
    tabs("reports", [
      ["pl", "損益計算書"],
      ["bs", "貸借対照表"],
      ["tb", "試算表"],
      ["ledger", "総勘定元帳"],
    ]),
    h(
      "div",
      { class: "toolbar" },
      sub !== "bs"
        ? h(
            "label",
            {},
            "期間",
            h("input", { type: "date", value: o.from, onchange: set("from") }),
            "〜",
          )
        : h("label", {}, "基準日"),
      h("input", { type: "date", value: o.to, onchange: set("to") }),
      sub === "ledger" ? accountSelect(() => true, o.account) : null,
      h("span", { class: "spacer" }),
      h("button", { onclick: exportCsv }, "仕訳CSVを出力"),
      h("button", { onclick: backup }, "SQLiteバックアップ"),
    ),
  );
  if (sub === "ledger")
    head.querySelector(".toolbar select").addEventListener("change", set("account", Number));
  const amounts = (rows) =>
    rows.map((r) => h("tr", {}, h("td", {}, `${r.code} ${r.name}`), num(r.amount)));
  if (sub === "pl") {
    const r = await api("report", bookArgs({ kind: "income_statement", from: o.from, to: o.to }));
    return h(
      "div",
      {},
      head,
      h(
        "div",
        { class: "grid2" },
        h(
          "div",
          { class: "card" },
          h("h2", {}, "収益"),
          table(
            ["科目", "金額"],
            [
              ...amounts(r.income),
              h("tr", { class: "total" }, h("td", {}, "収益合計"), num(r.total_income)),
            ],
            { numeric: [1] },
          ),
        ),
        h(
          "div",
          { class: "card" },
          h("h2", {}, "費用"),
          table(
            ["科目", "金額"],
            [
              ...amounts(r.expense),
              h("tr", { class: "total" }, h("td", {}, "費用合計"), num(r.total_expense)),
            ],
            { numeric: [1] },
          ),
        ),
      ),
      h(
        "div",
        { class: "tiles" },
        tile(isBusiness() ? "所得（収益−費用）" : "収支（収入−支出）", r.net_income),
      ),
    );
  }
  if (sub === "bs") {
    const r = await api("report", bookArgs({ kind: "balance_sheet", to: o.to }));
    return h(
      "div",
      {},
      head,
      h(
        "div",
        { class: "grid2" },
        h(
          "div",
          { class: "card" },
          h("h2", {}, "資産"),
          table(
            ["科目", "金額"],
            [
              ...amounts(r.assets),
              h("tr", { class: "total" }, h("td", {}, "資産合計"), num(r.total_assets)),
            ],
            { numeric: [1] },
          ),
        ),
        h(
          "div",
          { class: "card" },
          h("h2", {}, "負債・純資産"),
          table(
            ["科目", "金額"],
            [
              ...amounts(r.liabilities),
              h("tr", { class: "total" }, h("td", {}, "負債合計"), num(r.total_liabilities)),
              ...amounts(r.equity),
              h("tr", {}, h("td", {}, "損益の累計（未振替）"), num(r.retained_earnings)),
              h(
                "tr",
                { class: "total" },
                h("td", {}, "負債・純資産合計"),
                num(decText(dec(r.total_liabilities) + dec(r.total_equity))),
              ),
            ],
            { numeric: [1] },
          ),
          h(
            "p",
            { class: r.balanced ? "pos" : "neg" },
            r.balanced ? "✓ 貸借が一致しています" : "貸借が一致しません",
          ),
        ),
      ),
    );
  }
  if (sub === "tb") {
    const r = await api("report", bookArgs({ kind: "trial_balance", from: o.from, to: o.to }));
    return h(
      "div",
      {},
      head,
      h(
        "div",
        { class: "card" },
        table(
          ["コード", "科目", "区分", "期首残高", "借方", "貸方", "期末残高"],
          [
            ...r.rows
              .filter((x) => !(isZero(x.opening) && isZero(x.debit) && isZero(x.credit)))
              .map((x) =>
                h(
                  "tr",
                  {},
                  h("td", {}, x.code),
                  h("td", {}, x.name),
                  h("td", {}, label("categories", x.category)),
                  num(x.opening),
                  num(x.debit),
                  num(x.credit),
                  num(x.closing),
                ),
              ),
            h(
              "tr",
              { class: "total" },
              h("td", {}),
              h("td", {}, "合計"),
              h("td"),
              h("td"),
              num(r.total_debit),
              num(r.total_credit),
              h("td"),
            ),
          ],
          { numeric: [3, 4, 5, 6] },
        ),
      ),
    );
  }
  if (!o.account) return h("div", {}, head, h("p", {}, "科目を選んでください。"));
  const r = await api(
    "report",
    bookArgs({ kind: "ledger", account: o.account, from: o.from, to: o.to }),
  );
  return h(
    "div",
    {},
    head,
    h(
      "div",
      { class: "card" },
      h("h2", {}, r.name),
      table(
        ["日付", "No.", "摘要", "相手科目", "借方", "貸方", "残高"],
        [
          h(
            "tr",
            {},
            h("td", {}, o.from),
            h("td"),
            h("td", {}, "前期繰越"),
            h("td"),
            h("td"),
            h("td"),
            num(r.opening),
          ),
          ...r.rows.map((x) =>
            h(
              "tr",
              {},
              h("td", {}, x.date),
              h("td", { class: "num" }, x.entry),
              h("td", {}, x.description, x.memo ? h("div", { class: "muted" }, x.memo) : null),
              h("td", {}, x.counter),
              num(isZero(x.debit) ? "" : x.debit),
              num(isZero(x.credit) ? "" : x.credit),
              num(x.balance),
            ),
          ),
        ],
        { numeric: [1, 4, 5, 6] },
      ),
    ),
  );
};
async function exportCsv() {
  const o = S.reportOptions;
  const r = await api("csv_export", bookArgs({ from: o.from, to: o.to }));
  download(new Blob([r.content], { type: r.mime_type }), `${S.book.name}-仕訳.csv`);
}
async function backup() {
  const r = await fetch("/web/backup", {
    method: "POST",
    headers: { "content-type": "application/json", "x-yy-csrf": S.csrf },
    body: "{}",
  });
  if (!r.ok) return message((await r.json()).error, true);
  download(await r.blob(), "yyaccountant-backup.sqlite3");
}
function download(blob, name) {
  const a = h("a", { href: URL.createObjectURL(blob), download: name });
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}

// ----- Audit -----
RENDER.audit = async () => {
  const r = await api("audit_list", bookArgs({ after: Math.max(0, S.auditAfter || 0) }));
  return h(
    "div",
    {},
    h("p", { class: "muted" }, "すべての変更は版番号と操作者とともに記録されます（100件ずつ）。"),
    h(
      "div",
      { class: "card" },
      table(
        ["版", "日時", "操作者", "操作", "内容"],
        r.changes.map((c) =>
          h(
            "tr",
            {},
            h("td", { class: "num" }, c.revision),
            h("td", {}, new Date(c.at).toLocaleString("ja-JP")),
            h("td", {}, c.actor),
            h("td", {}, c.change.operation),
            h(
              "td",
              {},
              h(
                "details",
                {},
                h("summary", {}, "詳細"),
                h("pre", { class: "pre" }, JSON.stringify(c.change, null, 1)),
              ),
            ),
          ),
        ),
      ),
      h(
        "div",
        { class: "toolbar", style: "margin-top:8px" },
        h(
          "button",
          {
            disabled: !S.auditAfter,
            onclick: () => ((S.auditAfter = Math.max(0, (S.auditAfter || 0) - 100)), render()),
          },
          "前へ",
        ),
        h(
          "button",
          {
            disabled: r.changes.length < 100,
            onclick: () => ((S.auditAfter = r.changes[r.changes.length - 1].id), render()),
          },
          "次へ",
        ),
      ),
    ),
  );
};

// ----- Book management -----
function newBook() {
  const f = fields([
    {
      key: "name",
      label: "帳簿名",
      required: true,
      placeholder: "例: わが家の家計 / ○○デザイン事務所",
    },
    {
      key: "kind",
      label: "種類",
      type: "select",
      value: "household",
      options: S.meta.kinds,
      help: "家計: 生活費の管理向けの科目。個人事業主: 青色申告決算書に沿った科目（事業主貸・事業主借など）。",
    },
    { key: "currency", label: "通貨", value: "JPY" },
    {
      key: "minor_digits",
      label: "小数桁",
      type: "number",
      value: 0,
      help: "円は0、ドル・ユーロは2",
    },
    {
      key: "fiscal_start_month",
      label: "期首月",
      type: "number",
      value: 1,
      help: "個人事業主の会計期間は1月〜12月です",
    },
  ]);
  modal("新しい帳簿", f.element, {
    onSubmit: async () => {
      const v = f.values();
      const b = await api("book_create", {
        name: v.name,
        kind: v.kind,
        currency: v.currency.toUpperCase(),
        minor_digits: v.minor_digits ?? 0,
        fiscal_start_month: v.fiscal_start_month ?? 1,
      });
      await loadBooks(b.id);
      render();
    },
  });
}
function bookSettings() {
  const b = S.book;
  const f = fields([
    { key: "name", label: "帳簿名", value: b.name, required: true },
    { key: "currency", label: "通貨", value: b.currency },
    { key: "minor_digits", label: "小数桁", type: "number", value: b.minor_digits },
    {
      key: "rounding",
      label: "端数処理",
      type: "select",
      value: b.rounding,
      options: S.meta.rounding,
      help: "利息・按分・償却などの計算結果に適用します",
    },
    { key: "fiscal_start_month", label: "期首月", type: "number", value: b.fiscal_start_month },
  ]);
  modal("帳簿設定", f.element, {
    onSubmit: async () => {
      const v = f.values();
      await change({
        operation: "update_book",
        name: v.name,
        currency: v.currency.toUpperCase(),
        minor_digits: v.minor_digits,
        rounding: v.rounding,
        fiscal_start_month: v.fiscal_start_month,
      });
      await loadBooks(b.id);
      render();
    },
  });
}

boot().catch((e) => {
  $("#login").hidden = false;
  $("#login-error").textContent = e.message;
});
