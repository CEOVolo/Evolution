// Legible Canvas prototype over the deterministic sim-core (wasm).
// Rough on purpose (Canvas2D, Phase-0 genome — no neural brains yet), but self-explanatory.

import init, { Sim } from "../pkg/evolution.js";

async function main() {
  await init();
  const sim = new Sim((Math.random() * 0xffffffff) >>> 0);

  const worldW = sim.world_w();
  const worldH = sim.world_h();
  const gridW = sim.grid_w();
  const gridH = sim.grid_h();
  const cellW = worldW / gridW;
  const cellH = worldH / gridH;

  const canvas = document.getElementById("c");
  const ctx = canvas.getContext("2d");
  const sxk = canvas.width / worldW;
  const syk = canvas.height / worldH;

  const fieldCanvas = document.createElement("canvas");
  fieldCanvas.width = gridW;
  fieldCanvas.height = gridH;
  const fctx = fieldCanvas.getContext("2d");
  const fimg = fctx.createImageData(gridW, gridH);

  const $ = (id) => document.getElementById(id);

  // --- controls ---------------------------------------------------------
  let playing = true;
  let speed = 4;
  let brush = "food";

  const playBtn = $("play");
  playBtn.onclick = () => {
    playing = !playing;
    playBtn.textContent = playing ? "⏸ Пауза" : "▶ Играть";
    playBtn.classList.toggle("primary", playing);
  };
  $("reset").onclick = () => {
    sim.reset((Math.random() * 0xffffffff) >>> 0);
    popHist.length = 0;
    toast("🌍 Новый мир засеян");
  };
  $("speed").oninput = (e) => (speed = +e.target.value);
  $("brush").onchange = (e) => (brush = e.target.value);
  $("mut").oninput = (e) => sim.set_mutation_rate(+e.target.value);
  $("regrow").oninput = (e) => sim.set_field_regrow(+e.target.value);
  $("eat").oninput = (e) => sim.set_eat_rate(+e.target.value);

  function worldFromEvent(e) {
    const rect = canvas.getBoundingClientRect();
    const wx = ((e.clientX - rect.left) / rect.width) * worldW;
    const wy = ((e.clientY - rect.top) / rect.height) * worldH;
    return [wx, wy];
  }
  function applyBrush(e) {
    const [wx, wy] = worldFromEvent(e);
    const cx = Math.floor(wx / cellW);
    const cy = Math.floor(wy / cellH);
    if (brush === "food") sim.inject(cx, cy, 3, 900);
    else if (brush === "spawn") for (let k = 0; k < 6; k++) sim.spawn(cx, cy, 200);
    else if (brush === "kill") sim.kill(cx - 3, cy - 3, cx + 3, cy + 3);
  }
  canvas.addEventListener("mousedown", applyBrush);
  canvas.addEventListener("mousemove", (e) => {
    if (e.buttons & 1) applyBrush(e);
    else inspectHover(e);
  });
  canvas.addEventListener("contextmenu", (e) => e.preventDefault());

  // --- inspector (hover) ------------------------------------------------
  const inspectBody = $("inspect-body");
  let lastInspect = 0;
  function inspectHover(e) {
    const now = performance.now();
    if (now - lastInspect < 70) return;
    lastInspect = now;
    const [wx, wy] = worldFromEvent(e);
    const n = sim.nearest(wx, wy);
    if (!n.length) {
      inspectBody.innerHTML = '<div class="empty">Здесь пусто.</div>';
      return;
    }
    const [, , energy, age, spd, metab, repro, r, g, b, id] = n;
    inspectBody.innerHTML = `
      <div class="row" style="margin:0 0 8px">
        <span><span class="swatch" style="background:rgb(${r|0},${g|0},${b|0})"></span> клетка #${id | 0}</span>
        <span class="mono" style="color:var(--muted2)">возраст ${age | 0}</span>
      </div>
      <div class="row mono" style="margin:4px 0"><span>энергия</span><b>${energy | 0}</b></div>
      <div class="row mono" style="margin:4px 0"><span>ген «скорость»</span><b>${spd.toFixed(2)}</b></div>
      <div class="row mono" style="margin:4px 0"><span>ген «обмен»</span><b>${metab.toFixed(2)}</b></div>
      <div class="row mono" style="margin:4px 0"><span>ген «размножение»</span><b>${repro.toFixed(2)}</b></div>`;
  }

  // --- population chart + trait bars ------------------------------------
  const chart = $("chart");
  const cctx = chart.getContext("2d");
  const popHist = [];
  const POP_MAX_SAMPLES = 272;

  function drawChart() {
    cctx.clearRect(0, 0, chart.width, chart.height);
    if (popHist.length < 2) return;
    const max = Math.max(...popHist, 1);
    cctx.beginPath();
    for (let i = 0; i < popHist.length; i++) {
      const x = (i / (POP_MAX_SAMPLES - 1)) * chart.width;
      const y = chart.height - (popHist[i] / max) * (chart.height - 4) - 2;
      i ? cctx.lineTo(x, y) : cctx.moveTo(x, y);
    }
    cctx.strokeStyle = "#6ee7a0";
    cctx.lineWidth = 1.5;
    cctx.stroke();
    cctx.fillStyle = "#8299836b";
    cctx.font = "10px ui-monospace, monospace";
    cctx.fillText("макс " + max.toLocaleString(), 4, 11);
  }

  const bar = (id, frac) => {
    $("b-" + id).style.width = Math.max(0, Math.min(1, frac)) * 100 + "%";
  };
  function updateTraits() {
    const [spd, metab, repro] = sim.avg_traits();
    $("a-speed").textContent = spd.toFixed(3);
    $("a-metab").textContent = metab.toFixed(3);
    $("a-repro").textContent = repro.toFixed(3);
    bar("speed", spd); // 0..1
    bar("metab", (metab - 0.3) / (2.0 - 0.3));
    bar("repro", (repro - 0.5) / (1.5 - 0.5));
  }

  // --- toasts (plain-language narration) --------------------------------
  const toastsEl = $("toasts");
  function toast(text) {
    const d = document.createElement("div");
    d.className = "toast";
    d.textContent = text;
    toastsEl.appendChild(d);
    setTimeout(() => d.classList.add("fade"), 3200);
    setTimeout(() => d.remove(), 3900);
  }
  let lastNarr = performance.now();
  let lastNarrPop = sim.population();
  function narrate() {
    const now = performance.now();
    if (now - lastNarr < 2500) return;
    const pop = sim.population();
    const prev = lastNarrPop || 1;
    const ratio = pop / prev;
    if (pop === 0) toast("💀 Мир вымер — попробуй «Новый мир» или подсыпь еды");
    else if (ratio > 1.35) toast(`🌱 Вспышка размножения (+${(pop - prev).toLocaleString()})`);
    else if (ratio < 0.7) toast(`💀 Массовое вымирание (−${(prev - pop).toLocaleString()})`);
    lastNarr = now;
    lastNarrPop = pop;
  }

  // --- render loop ------------------------------------------------------
  const tickEl = $("s-tick");
  const popEl = $("s-pop");
  let frame = 0;

  function draw() {
    const f = sim.field();
    const d = fimg.data;
    for (let i = 0, j = 0; i < f.length; i++, j += 4) {
      const v = f[i];
      d[j] = 12;
      d[j + 1] = 26 + v * 0.62;
      d[j + 2] = 22;
      d[j + 3] = 255;
    }
    fctx.putImageData(fimg, 0, 0);
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(fieldCanvas, 0, 0, canvas.width, canvas.height);

    const p = sim.positions();
    const c = sim.colors();
    for (let i = 0, k = 0; i < p.length; i += 2, k += 3) {
      ctx.fillStyle = `rgb(${c[k]},${c[k + 1]},${c[k + 2]})`;
      ctx.fillRect(p[i] * sxk - 1.3, p[i + 1] * syk - 1.3, 2.6, 2.6);
    }
  }

  function loop() {
    if (playing) sim.tick(speed);
    draw();

    if ((frame & 3) === 0) {
      popHist.push(sim.population());
      if (popHist.length > POP_MAX_SAMPLES) popHist.shift();
      drawChart();
    }
    if ((frame & 7) === 0) {
      tickEl.textContent = sim.tick_count().toLocaleString();
      popEl.textContent = sim.population().toLocaleString();
      updateTraits();
      narrate();
    }
    frame++;
    requestAnimationFrame(loop);
  }

  // Debug/verification hook (rAF is throttled when the tab is backgrounded).
  window.__evo = {
    sim,
    step: (n) => {
      sim.tick(n);
      draw();
      updateTraits();
    },
  };

  toast("👋 Это живой мир. Наведи на точку, покрути ползунки, посыпь еды.");
  loop();
}

main();
