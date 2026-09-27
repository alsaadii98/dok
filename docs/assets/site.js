// dok.alsaadii98.com — progressive enhancement only. Every page is complete
// and readable with this file blocked; nothing here hides content first.
(() => {
  "use strict";

  const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const live = document.getElementById("live");
  const say = (msg) => {
    if (!live) return;
    live.textContent = "";
    requestAnimationFrame(() => (live.textContent = msg));
  };

  // ── copy ──────────────────────────────────────────────────────────────
  // Feedback: the button confirms, and the live region tells screen readers.
  document.addEventListener("click", async (ev) => {
    const btn = ev.target.closest("[data-copy]");
    if (!btn) return;
    const text = btn.dataset.copy;
    try {
      await navigator.clipboard.writeText(text);
      btn.dataset.state = "done";
      btn.textContent = "copied";
      say("Copied to clipboard");
      clearTimeout(btn._t);
      btn._t = setTimeout(() => {
        btn.textContent = "copy";
        delete btn.dataset.state;
      }, 1600);
    } catch {
      say("Copy failed. Select the text and copy it manually.");
    }
  });

  // ── tabs ──────────────────────────────────────────────────────────────
  // One implementation for the install tabs and the command browser. Arrow
  // keys move between tabs (WAI-ARIA tab pattern), and the selection is in
  // the URL hash so a specific command or install method can be linked.
  function tabs(root) {
    const list = root.querySelector('[role="tablist"]');
    const btns = [...root.querySelectorAll('[role="tab"]')];
    const prefix = root.dataset.hash || "";

    function select(btn, { focus = false, push = false } = {}) {
      for (const b of btns) {
        const on = b === btn;
        b.setAttribute("aria-selected", String(on));
        b.tabIndex = on ? 0 : -1;
        const panel = document.getElementById(b.getAttribute("aria-controls"));
        if (panel) panel.hidden = !on;
      }
      if (focus) btn.focus();
      if (push && prefix) {
        history.replaceState(null, "", `#${prefix}${btn.dataset.key}`);
      }
    }

    list.addEventListener("click", (ev) => {
      const b = ev.target.closest('[role="tab"]');
      if (b) select(b, { push: true });
    });
    list.addEventListener("keydown", (ev) => {
      const i = btns.indexOf(document.activeElement);
      if (i < 0) return;
      const vertical = list.getAttribute("aria-orientation") === "vertical";
      const next = vertical ? "ArrowDown" : "ArrowRight";
      const prev = vertical ? "ArrowUp" : "ArrowLeft";
      let j = null;
      if (ev.key === next) j = (i + 1) % btns.length;
      else if (ev.key === prev) j = (i - 1 + btns.length) % btns.length;
      else if (ev.key === "Home") j = 0;
      else if (ev.key === "End") j = btns.length - 1;
      if (j === null) return;
      ev.preventDefault();
      select(btns[j], { focus: true, push: true });
    });

    const fromHash = () => {
      if (!prefix || !location.hash.startsWith(`#${prefix}`)) return null;
      const key = location.hash.slice(prefix.length + 1);
      return btns.find((b) => b.dataset.key === key) || null;
    };
    select(fromHash()
      || btns.find((b) => b.getAttribute("aria-selected") === "true")
      || btns[0]);
    // An in-page link to #cmd-prune changes the hash without reloading, so
    // follow it here too, and bring the tabs into view.
    addEventListener("hashchange", () => {
      const b = fromHash();
      if (!b) return;
      select(b);
      root.scrollIntoView({ block: "start", behavior: reduce ? "auto" : "smooth" });
    });
  }
  document.querySelectorAll("[data-tabs]").forEach(tabs);

  // ── decode ────────────────────────────────────────────────────────────
  // Storytelling, once: the headline word resolves out of noise, which is the
  // product in one second. Skipped entirely under reduced motion.
  const word = document.querySelector("[data-decode]");
  if (word && !reduce) {
    const final = word.textContent;
    const glyphs = "#%&<>/{}[]=+*01";
    const steps = 18;
    let frame = 0;
    word.setAttribute("aria-label", final);
    const tick = () => {
      frame += 1;
      const settled = Math.floor((frame / steps) * final.length);
      word.textContent = [...final]
        .map((ch, i) => (i < settled ? ch : glyphs[(Math.random() * glyphs.length) | 0]))
        .join("");
      if (frame < steps) requestAnimationFrame(() => setTimeout(tick, 40));
      else word.textContent = final;
    };
    setTimeout(tick, 250);
  }

  // ── pause ─────────────────────────────────────────────────────────────
  // The casts are SMIL inside <img>, which cannot be paused, so pausing swaps
  // every cast for its still frame. WCAG 2.2.2: looping motion next to other
  // content needs a way to stop it. Reduced-motion users get the stills
  // without asking, via <picture>.
  const pause = document.querySelector("[data-pause]");
  if (pause) {
    const casts = [...document.querySelectorAll("img[data-still]")];
    if (reduce) pause.hidden = true;
    pause.addEventListener("click", () => {
      const on = pause.getAttribute("aria-pressed") !== "true";
      for (const img of casts) {
        if (!img.dataset.cast) img.dataset.cast = img.getAttribute("src");
        img.src = on ? img.dataset.still : img.dataset.cast;
      }
      pause.setAttribute("aria-pressed", String(on));
      pause.textContent = on ? "Play animations" : "Pause animations";
      say(on ? "Animations paused" : "Animations playing");
    });
  }

  // ── pipeline ──────────────────────────────────────────────────────────
  // Storytelling: the stages light in the order data actually flows through
  // them, from the socket to the terminal, the first time the diagram is seen.
  const pipe = document.querySelector("[data-pipeline]");
  if (pipe) {
    if (reduce || !("IntersectionObserver" in window)) {
      pipe.dataset.run = "done";
    } else {
      const io = new IntersectionObserver(
        (entries) => {
          if (!entries.some((e) => e.isIntersecting)) return;
          pipe.dataset.run = "go";
          io.disconnect();
        },
        { threshold: 0.35 }
      );
      io.observe(pipe);
    }
  }
})();
