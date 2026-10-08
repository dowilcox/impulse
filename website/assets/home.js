// Impulse homepage: the pulses that run along the hero's grid, scroll reveals,
// and the demo window settling flat as it scrolls into view.
(() => {
  "use strict";
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;

  // Top bar gets its hairline once the page scrolls.
  const topbar = document.querySelector(".topbar");
  const onScroll = () => topbar?.classList.toggle("scrolled", scrollY > 8);
  addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  // Reveal on scroll, staggered within each group.
  const reveals = [...document.querySelectorAll(".reveal")];
  if (reduced || !("IntersectionObserver" in window)) {
    reveals.forEach((el) => el.classList.add("in"));
  } else {
    const io = new IntersectionObserver((entries) => {
      entries.forEach((en) => {
        if (!en.isIntersecting) return;
        const siblings = [...en.target.parentElement.querySelectorAll(":scope > .reveal")];
        en.target.style.setProperty("--d", `${Math.max(0, siblings.indexOf(en.target)) * 0.07}s`);
        en.target.classList.add("in");
        io.unobserve(en.target);
      });
    }, { rootMargin: "0px 0px -8% 0px" });
    reveals.forEach((el) => io.observe(el));
  }

  // The demo window starts tilted back and lies flat as it comes up the page.
  const stage = document.querySelector("[data-demo-stage]");
  const win = stage?.querySelector(".win");
  if (win && !reduced) {
    let ticking = false;
    const tilt = () => {
      ticking = false;
      if (innerWidth < 760) { win.style.removeProperty("--tilt"); win.style.removeProperty("--scale"); return; }
      const top = stage.getBoundingClientRect().top;
      const p = Math.min(1, Math.max(0, (innerHeight - top) / (innerHeight * 0.75)));
      const e = 1 - Math.pow(1 - p, 3);
      win.style.setProperty("--tilt", `${(1 - e) * 22}deg`);
      win.style.setProperty("--scale", `${0.94 + e * 0.06}`);
    };
    addEventListener("scroll", () => { if (!ticking) { ticking = true; requestAnimationFrame(tilt); } }, { passive: true });
    addEventListener("resize", tilt);
    tilt();
  }

  // Pulses: short bright streaks that travel along the 48px grid lines.
  const canvas = document.querySelector(".pulses");
  if (!canvas || reduced) return;
  const ctx = canvas.getContext("2d");
  const GRID = 48;
  const COLORS = ["125,207,255", "122,162,247", "187,154,247", "158,206,106"];
  let pulses = [];
  let w = 0, h = 0, dpr = 1, running = false, last = 0, spawnIn = 0;

  function resize() {
    dpr = Math.min(2, devicePixelRatio || 1);
    w = canvas.clientWidth; h = canvas.clientHeight;
    canvas.width = w * dpr; canvas.height = h * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  }

  function spawn() {
    const horizontal = Math.random() < 0.6;
    // Lines sit where the CSS grid draws them (background centered on the hero).
    const offsetX = (w / 2) % GRID;
    const speed = 160 + Math.random() * 260;
    const dir = Math.random() < 0.5 ? 1 : -1;
    const color = COLORS[(Math.random() * COLORS.length) | 0];
    if (horizontal) {
      const y = GRID * (1 + ((Math.random() * (h / GRID - 1)) | 0));
      pulses.push({ horizontal, line: y, pos: dir > 0 ? -80 : w + 80, dir, speed, len: 80 + Math.random() * 120, color });
    } else {
      const x = offsetX + GRID * ((Math.random() * (w / GRID)) | 0);
      pulses.push({ horizontal, line: x, pos: dir > 0 ? -80 : h + 80, dir, speed, len: 60 + Math.random() * 100, color });
    }
  }

  function frame(t) {
    if (!running) return;
    const dt = Math.min(0.05, (t - last) / 1000 || 0);
    last = t;
    spawnIn -= dt;
    if (spawnIn <= 0 && pulses.length < 18) { spawn(); spawnIn = 0.18 + Math.random() * 0.35; }
    ctx.clearRect(0, 0, w, h);
    pulses = pulses.filter((p) => {
      p.pos += p.dir * p.speed * dt;
      const end = p.horizontal ? w : h;
      if (p.pos < -p.len - 100 || p.pos > end + p.len + 100) return false;
      const head = p.pos, tail = p.pos - p.dir * p.len;
      const g = p.horizontal ? ctx.createLinearGradient(tail, 0, head, 0) : ctx.createLinearGradient(0, tail, 0, head);
      g.addColorStop(0, `rgba(${p.color},0)`);
      g.addColorStop(1, `rgba(${p.color},0.9)`);
      ctx.strokeStyle = g;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      if (p.horizontal) { ctx.moveTo(tail, p.line + 0.5); ctx.lineTo(head, p.line + 0.5); }
      else { ctx.moveTo(p.line + 0.5, tail); ctx.lineTo(p.line + 0.5, head); }
      ctx.stroke();
      ctx.fillStyle = `rgba(${p.color},1)`;
      ctx.shadowColor = `rgba(${p.color},0.9)`;
      ctx.shadowBlur = 10;
      ctx.beginPath();
      if (p.horizontal) ctx.arc(head, p.line + 0.5, 1.6, 0, Math.PI * 2);
      else ctx.arc(p.line + 0.5, head, 1.6, 0, Math.PI * 2);
      ctx.fill();
      ctx.shadowBlur = 0;
      return true;
    });
    requestAnimationFrame(frame);
  }

  const start = () => { if (!running) { running = true; last = performance.now(); requestAnimationFrame(frame); } };
  const stop = () => { running = false; };
  resize();
  addEventListener("resize", resize);
  new IntersectionObserver(([en]) => (en.isIntersecting && !document.hidden ? start() : stop())).observe(canvas);
  document.addEventListener("visibilitychange", () => (document.hidden ? stop() : start()));
})();
