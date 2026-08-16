/* COPY DIFF — a unified diff of the current floor (canonical JSON) against
   the text it was loaded from, copied to the clipboard so it can be pasted
   into a chat / applied with `patch -p1` (paths are a/levels/… →
   b/levels/…). No dependencies: line LCS + hunks with 3 lines of context.
   Floors are a few hundred lines, so the O(n·m) LCS table is trivial. */
(function () {
  "use strict";

  function lcsDiff(a, b) {
    // returns ops: [{t:' '|'-'|'+', s:line}]
    const n = a.length, m = b.length;
    const dp = new Array(n + 1);
    for (let i = 0; i <= n; i++) dp[i] = new Uint16Array(m + 1);
    for (let i = n - 1; i >= 0; i--)
      for (let j = m - 1; j >= 0; j--)
        dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    const ops = [];
    let i = 0, j = 0;
    while (i < n && j < m) {
      if (a[i] === b[j]) { ops.push({ t: " ", s: a[i] }); i++; j++; }
      else if (dp[i + 1][j] >= dp[i][j + 1]) { ops.push({ t: "-", s: a[i] }); i++; }
      else { ops.push({ t: "+", s: b[j] }); j++; }
    }
    while (i < n) ops.push({ t: "-", s: a[i++] });
    while (j < m) ops.push({ t: "+", s: b[j++] });
    return ops;
  }

  function unified(oldText, newText, path, ctx) {
    ctx = ctx == null ? 3 : ctx;
    const a = oldText.replace(/\n$/, "").split("\n");
    const b = newText.replace(/\n$/, "").split("\n");
    const ops = lcsDiff(a, b);
    if (!ops.some((o) => o.t !== " ")) return "";
    const out = [`--- a/${path}`, `+++ b/${path}`];
    // group into hunks
    let k = 0;
    let oldLine = 1, newLine = 1; // 1-based positions at ops[k]
    while (k < ops.length) {
      // find next change
      while (k < ops.length && ops[k].t === " ") { k++; oldLine++; newLine++; }
      if (k >= ops.length) break;
      let start = Math.max(0, k - ctx);
      // hunk extends while changes are within 2*ctx of each other
      let end = k;
      let last = k;
      while (end < ops.length) {
        if (ops[end].t !== " ") last = end;
        else if (end - last > 2 * ctx) break;
        end++;
      }
      end = Math.min(ops.length, last + ctx + 1);
      // compute hunk header numbers
      const backCtx = k - start;
      const oStart = oldLine - backCtx, nStart = newLine - backCtx;
      let oLen = 0, nLen = 0;
      const body = [];
      for (let q = start; q < end; q++) {
        const o = ops[q];
        body.push(o.t + o.s);
        if (o.t !== "+") oLen++;
        if (o.t !== "-") nLen++;
      }
      out.push(`@@ -${oStart},${oLen} +${nStart},${nLen} @@`);
      out.push(...body);
      // advance counters past this hunk
      for (let q = k; q < end; q++) { if (ops[q].t !== "+") oldLine++; if (ops[q].t !== "-") newLine++; }
      k = end;
    }
    return out.join("\n") + "\n";
  }

  async function copyText(text) {
    try { await navigator.clipboard.writeText(text); return true; } catch (e) { return false; }
  }

  function showModal(title, text) {
    let m = document.getElementById("diff-modal");
    if (!m) {
      m = document.createElement("div");
      m.id = "diff-modal";
      m.style.cssText = "position:fixed;inset:0;background:rgba(0,0,0,.72);z-index:50;display:flex;align-items:center;justify-content:center;padding:24px;";
      m.innerHTML = '<div style="background:#0e0b1a;border:1px solid #2a2050;border-radius:6px;width:min(900px,100%);max-height:100%;display:flex;flex-direction:column;box-shadow:0 0 40px -10px #37f0e6">' +
        '<div style="display:flex;align-items:center;gap:10px;padding:8px 12px;border-bottom:1px solid #2a2050;color:#8a82c0;letter-spacing:2px"><span id="diff-title"></span><span style="flex:1"></span>' +
        '<button id="diff-copy">COPY</button><button id="diff-close">CLOSE</button></div>' +
        '<textarea id="diff-text" spellcheck="false" style="flex:1;min-height:360px;margin:10px;font-size:15px;line-height:1.2;white-space:pre;overflow:auto"></textarea></div>';
      document.body.appendChild(m);
      m.querySelector("#diff-close").onclick = () => (m.style.display = "none");
      m.querySelector("#diff-copy").onclick = async () => {
        const ta = m.querySelector("#diff-text");
        const ok = await copyText(ta.value);
        if (!ok) { ta.focus(); ta.select(); document.execCommand && document.execCommand("copy"); }
      };
      m.addEventListener("click", (e) => { if (e.target === m) m.style.display = "none"; });
    }
    m.querySelector("#diff-title").textContent = title;
    m.querySelector("#diff-text").value = text;
    m.style.display = "flex";
  }

  function currentDiff() {
    const ed = window.__editor;
    if (!ed || !ed.state || !ed.state.cur) return { path: null, text: "" };
    const cur = ed.state.cur;
    // The editor's dir is repo-root relative ("levels/" or "levels/samples/"),
    // so the diff paths work with `patch -p1` from the repo root.
    const dir = (ed.state.dir || "levels/").replace(/^\.?\//, "").replace(/\/?$/, "/");
    const path = dir + cur.file;
    const before = cur.savedText == null ? "" : cur.savedText;
    const after = ed.canonical();
    return { path, text: unified(before, after, path) };
  }

  function wire() {
    const anchor = document.getElementById("btn-download");
    if (!anchor || document.getElementById("btn-diff")) return;
    const b = document.createElement("button");
    b.id = "btn-diff";
    b.title = "Unified diff of this floor vs. what was loaded — copied to the clipboard (paste it to Claude, or `patch -p1`)";
    b.textContent = "COPY DIFF";
    anchor.parentNode.insertBefore(b, anchor);
    b.onclick = async () => {
      const { path, text } = currentDiff();
      if (!path) return;
      const st = document.getElementById("status");
      if (!text) { if (st) { st.textContent = "no changes vs. loaded " + path; st.className = ""; } return; }
      const ok = await copyText(text);
      if (st) { st.textContent = (ok ? "diff copied — " : "diff ready — ") + path; st.className = ok ? "ok" : ""; }
      showModal((ok ? "COPIED · " : "") + path, text);
    };
  }

  window.__editorDiff = { unified, currentDiff };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", wire); else wire();
})();
