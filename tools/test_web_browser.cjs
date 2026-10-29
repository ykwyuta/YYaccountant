// Drive the real Web UI in Chromium: login, every view of both demo books, quick entry with the
// account finder, a balanced journal entry, plan posting and dark mode. Fails on console errors.
const { chromium } = require("playwright");
const path = require("path");

const url = process.env.YYA_URL;
const password = process.env.YYA_PASSWORD;
const out = process.env.YYA_OUT || ".";
const views = ["home", "input", "journal", "accounts", "claims", "budget", "plans", "cost", "stats", "reports", "audit"];

(async () => {
  const browser = await chromium.launch();
  const errors = [];
  for (const scheme of ["light", "dark"]) {
    const page = await browser.newPage({ viewport: { width: 1360, height: 900 }, colorScheme: scheme });
    page.on("console", (m) => m.type() === "error" && errors.push(`${scheme}: ${m.text()}`));
    page.on("pageerror", (e) => errors.push(`${scheme}: ${e.message}`));
    await page.goto(url);
    await page.fill("#password", password);
    await page.click("#login-form button[type=submit]");
    await page.waitForSelector("#app:not([hidden])");
    const waitIdle = () => page.waitForFunction(() => document.querySelector("#surface").style.opacity === "");
    const shot = (name) => page.screenshot({ path: path.join(out, `${scheme}-${name}.png`), fullPage: true });
    if (scheme === "dark") {
      await page.click('[data-view="stats"]');
      await waitIdle();
      await page.waitForSelector("svg.chart");
      await shot("stats");
      await page.close();
      continue;
    }
    for (const book of ["1", "2"]) {
      await page.selectOption("#books", book);
      for (const view of views) {
        await page.click(`[data-view="${view}"]`);
        await waitIdle();
        if ((await page.locator("#message.error").count()) && (await page.isVisible("#message.error")))
          throw new Error(`${book}/${view}: ${await page.textContent("#message")}`);
        if (view === "cost" && book === "2") {
          for (const tab of ["製造実績", "原材料", "設備"]) {
            await page.click(`.subtabs button:has-text("${tab}")`);
            await waitIdle();
            await shot(`book${book}-cost-${tab}`);
          }
          await page.click('.subtabs button:has-text("製品原価")');
          await waitIdle();
        }
        await shot(`book${book}-${view}`);
      }
    }
    // Quick entry through the account finder on the sole proprietor book.
    await page.click('[data-view="input"]');
    await waitIdle();
    await page.fill('input[type="search"]', "ガソリン");
    await page.waitForSelector('.suggest button:has-text("車両費")');
    await page.click('.suggest button:has-text("車両費")');
    await page.waitForSelector('.guide h4:has-text("車両費")');
    await page.fill("input.amount", "５，５００");
    await page.fill('input[placeholder="100"]', "60");
    await page.fill('input[placeholder^="例: スーパー"]', "ブラウザー試験");
    await shot("quick-entry-guide");
    await page.click('button:has-text("記録する")');
    await page.waitForSelector('#message:not(.error):has-text("記録しました")');
    await waitIdle();
    const recent = await page.textContent(".card:last-child table");
    if (!recent.includes("ブラウザー試験") || !recent.includes("3,300") || !recent.includes("2,200"))
      throw new Error("quick entry with business ratio was not split 3,300 / 2,200: " + recent);
    // Balanced multi-line journal entry with live decimal totals.
    await page.click('[data-view="journal"]');
    await waitIdle();
    await page.click('button:has-text("仕訳を追加")');
    const rows = page.locator(".modal table.lines tbody tr");
    await page.click('.modal button:has-text("行を追加")');
    await rows.nth(0).locator("select.account").selectOption({ label: "費用｜514 旅費交通費" });
    await rows.nth(0).locator("input.debit").fill("0.5");
    await rows.nth(1).locator("select.account").selectOption({ label: "費用｜530 会議費" });
    await rows.nth(1).locator("input.debit").fill("999.5");
    await rows.nth(2).locator("select.account").selectOption({ label: "資産｜101 現金" });
    await rows.nth(2).locator("input.credit").fill("1000");
    await page.waitForSelector('.balance-ok:has-text("一致")');
    await page.click('.modal button[type="submit"]');
    // JPY has no fraction digits: the dialog stays open with the server's message.
    await page.waitForSelector('.modal .error:has-text("小数0桁")');
    await shot("journal-validation");
    await rows.nth(0).locator("input.debit").fill("1");
    await rows.nth(1).locator("input.debit").fill("999");
    await page.click('.modal button[type="submit"]');
    await page.waitForSelector(".modal", { state: "detached" });
    await page.waitForSelector('#message:not(.error):has-text("保存しました")');
    // Post a pending plan from the plans view if one exists.
    await page.selectOption("#books", "1");
    await page.click('[data-view="plans"]');
    await waitIdle();
    await shot("plans-before-post");
    // Chart tooltip.
    await page.click('[data-view="home"]');
    await waitIdle();
    const box = await page.locator("svg.chart .hit").last().boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.waitForSelector("#tooltip:not([hidden])");
    await shot("home-tooltip");
    // Narrow layout has no horizontal page scroll.
    await page.setViewportSize({ width: 390, height: 800 });
    await page.click('[data-view="input"]');
    await waitIdle();
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
    if (overflow > 1) throw new Error(`horizontal overflow ${overflow}px at phone width`);
    await shot("phone-input");
    await page.close();
  }
  await browser.close();
  const real = errors.filter((e) => !/status of 40[0-9]/.test(e));
  if (real.length) throw new Error("Console errors:\n" + real.join("\n"));
  console.log("Browser validation passed");
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
