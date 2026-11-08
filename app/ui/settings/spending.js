"use strict";

// Spending tab: what cloud providers have cost
// (estimated, from Rust's spend ledger) and the budget limits.

/** Dollars, with enough digits that a single dictation isn't "$0.00". */
function money(usd) {
  if (!usd) return "$0.00";
  return usd < 0.01 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`;
}

function renderSpending() {
  const sp = state.spending;
  if (!sp) return;
  const month = sp.month;
  $("spend-month").textContent = money(month.cost_usd);
  const per = month.requests ? ` · about ${money(month.cost_usd / month.requests)} each` : "";
  const unpriced = month.unpriced ? ` · ${month.unpriced} with an unknown price` : "";
  $("spend-month-detail").textContent = `${plural(month.requests, "paid request")}${per}${unpriced}`;
  $("spend-today").textContent = money(sp.today.cost_usd);

  const providers = Object.entries(month.by_provider).sort((a, b) => b[1] - a[1]);
  $("spend-providers").replaceChildren(
    ...providers.map(([name, cost]) => {
      const row = document.createElement("div");
      row.className = "row sub-row";
      row.innerHTML = `<span class="lbl"><small class="mono"></small></span><span class="mono"></span>`;
      row.querySelector("small").textContent = name;
      row.querySelector(".lbl + .mono").textContent = money(cost);
      return row;
    })
  );

  const b = state.settings.general.budget ?? {};
  setValue($("budget-month"), b.monthly_usd ?? "");
  setValue($("budget-day"), b.daily_usd ?? "");
  $("budget-banner").hidden = sp.state !== "reached";
  $("spend-month").classList.toggle("warn-text", sp.state !== "fine");
  $("spend-prices").textContent = `Prices as of ${sp.prices_as_of}; OpenRouter reports exact costs.`;
}

/** The budget fields as settings (empty or zero means no limit). */
function collectBudget() {
  const limit = (id) => {
    const v = parseFloat($(id).value);
    return Number.isFinite(v) && v > 0 ? v : null;
  };
  return { monthly_usd: limit("budget-month"), daily_usd: limit("budget-day") };
}

document.querySelectorAll("[data-limits]").forEach((b) =>
  b.addEventListener("click", () =>
    call("open_limits_page", { provider: b.dataset.limits })
  )
);
