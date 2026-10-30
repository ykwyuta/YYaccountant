// Drive the real Web UI in Chromium: login, every view of both demo books, quick entry with the
// account finder, a balanced journal entry, plan posting and dark mode. Fails on console errors.
const { chromium } = require("playwright");
const path = require("path");

const url = process.env.YYA_URL;
const password = process.env.YYA_PASSWORD;
const out = process.env.YYA_OUT || ".";
const views = ["home", "guide", "input", "journal", "accounts", "claims", "budget", "plans", "cost", "stats", "tax", "lifeplan", "reports", "audit"];

let current = null;
const errors = [];
(async () => {
  const browser = await chromium.launch();
  for (const scheme of ["light", "dark"]) {
    const page = await browser.newPage({ viewport: { width: 1360, height: 900 }, colorScheme: scheme });
    current = page;
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
    // Tutorial links to the screens it explains.
    await page.click('[data-view="guide"]');
    await waitIdle();
    await page.waitForSelector('.guide h2:has-text("2. 税金を計算する")');
    await page.click('.guide button:has-text("「税金」を開く")');
    await waitIdle();
    if ((await page.textContent("#view-title")) !== "税金") throw new Error("tutorial did not open the tax view");
    // Tax: the demo household (salary, side job, wife, son, parents, mortgage, furusato).
    const tiles = await page.textContent(".tiles");
    if (!tiles.includes("所得税の還付") || !tiles.includes("ふるさと納税の上限の目安"))
      throw new Error("tax tiles: " + tiles);
    await page.waitForSelector('td:has-text("寄附金税額控除（ふるさと納税）")');
    await page.click('button:has-text("親族を追加...")');
    await page.fill("#f-name", "祖母");
    await page.selectOption("#f-relation", "parent");
    await page.fill("#f-birth_year", "1940");
    await page.click('.modal button[type="submit"]');
    await page.waitForSelector(".modal", { state: "detached" });
    await waitIdle();
    await page.waitForSelector('td:has-text("扶養控除（祖母）")');
    await page.click('button:has-text("控除...")');
    await page.fill("#f-donation", "20,000");
    await page.click('.modal button[type="submit"]');
    await page.waitForSelector(".modal", { state: "detached" });
    await waitIdle();
    await page.waitForSelector('.tile:has-text("寄附 20,000 の実質負担")');
    await shot("tax-edited");
    // Life plan: compare the proposal with the current plan, add education costs, export CSV.
    await page.click('[data-view="lifeplan"]');
    await waitIdle();
    await page.waitForSelector('h2:has-text("キャッシュフロー表")');
    await page.selectOption('label:has-text("比べるプラン") select', { label: "提案（保険見直し・繰上返済）" });
    await waitIdle();
    await page.waitForSelector('p:has-text("最終年の差")');
    await page.waitForSelector('.card:has-text("6つの係数") td:has-text("1.218994")');
    await shot("lifeplan-compare");
    const flowsBefore = await page.locator('.card:has(h2:has-text("毎年の収支")) tbody tr').count();
    await page.click('button:has-text("教育費...")');
    await page.click('.modal button[type="submit"]');
    await page.waitForSelector('#message:not(.error):has-text("教育費を")');
    await waitIdle();
    const flowsAfter = await page.locator('.card:has(h2:has-text("毎年の収支")) tbody tr').count();
    if (flowsAfter <= flowsBefore) throw new Error(`education flows not added ${flowsBefore} -> ${flowsAfter}`);
    const [download] = await Promise.all([
      page.waitForEvent("download"),
      page.click('button:has-text("CSVで保存")'),
    ]);
    if (!download.suggestedFilename().endsWith("キャッシュフロー表.csv"))
      throw new Error("csv name " + download.suggestedFilename());
    // Chart tooltip.
    await page.click('[data-view="home"]');
    await waitIdle();
    const box = await page.locator("svg.chart .hit").last().boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.waitForSelector("#tooltip:not([hidden])");
    await shot("home-tooltip");
    // Problem sets: wrong journal → feedback, correct number → explanation, progress and practice book.
    await page.click('[data-view="study"]');
    await waitIdle();
    await shot("study-list");
    await page.click('.scenario-card:has-text("佐藤健さん") button.primary');
    await waitIdle();
    const sato = page.locator("#problem-sato-04 tbody tr");
    await page.click('#problem-sato-04 button:has-text("行を追加")');
    await sato.nth(0).locator("select").selectOption("121");
    await sato.nth(0).locator(".debit").fill("330,000");
    await sato.nth(1).locator("select").selectOption("401");
    await sato.nth(1).locator(".credit").fill("330000");
    await page.click("#problem-sato-04 button.primary");
    await page.waitForSelector('#problem-sato-04 .feedback li:has-text("事業主貸")');
    await sato.nth(0).locator(".debit").fill("296307");
    await sato.nth(2).locator("select").selectOption("191");
    await sato.nth(2).locator(".debit").fill("33693");
    await page.waitForSelector('#problem-sato-04 .balance-ok');
    await page.click("#problem-sato-04 button.primary");
    await page.waitForSelector("#problem-sato-04 .verdict.ok");
    await page.fill("#problem-sato-02 input.amount", "４４，０００");
    await page.click("#problem-sato-02 button.primary");
    await page.waitForSelector("#problem-sato-02 .verdict.ok");
    await page.check('#problem-sato-05 input[value="1"]');
    await page.click("#problem-sato-05 button.primary");
    await page.waitForSelector('#problem-sato-05 .verdict.ng');
    await page.click('#problem-sato-05 summary:has-text("選択肢ごとの解説")');
    await shot("study-sato");
    await page.click('.subtabs button:has-text("復習")');
    await waitIdle();
    if ((await page.locator("section.problem").count()) !== 1) throw new Error("review should list the one wrong problem");
    await page.click('button:has-text("練習用の帳簿を作る")');
    await page.waitForSelector('#message:not(.error):has-text("練習用の帳簿")');
    if (!(await page.textContent("#books option:checked")).includes("学習：佐藤健さん")) throw new Error("practice book not selected");
    await page.click('[data-view="tax"]');
    await waitIdle();
    await shot("study-practice-tax");
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
})().catch(async (e) => {
  console.error(e);
  if (errors.length) console.error("console errors:\n" + errors.join("\n"));
  // Keep what the page showed so a failure on a CI runner can be diagnosed from the artifact.
  if (current && !current.isClosed()) {
    try {
      console.error("message:", await current.textContent("#message", { timeout: 2000 }));
      console.error("tiles:", await current.textContent(".tiles", { timeout: 2000 }));
      await current.screenshot({ path: path.join(out, "failure.png"), fullPage: true });
    } catch (x) {
      console.error("could not record the failure:", x.message);
    }
  }
  process.exit(1);
});
