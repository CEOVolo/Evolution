// Canvas prototype over the deterministic sim-core (wasm), with a zoomable/pannable camera
// and follow mode — so you can watch individual cells interact, not just a global blur.

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

  const fieldCanvas = document.createElement("canvas");
  fieldCanvas.width = gridW;
  fieldCanvas.height = gridH;
  const fctx = fieldCanvas.getContext("2d");
  const fimg = fctx.createImageData(gridW, gridH);

  const $ = (id) => document.getElementById(id);

  // --- camera ---
  // The canvas is the full-viewport hero; size it to its box and fit the (square) world inside.
  let baseScale = 1;
  function resize() {
    const r = canvas.getBoundingClientRect();
    canvas.width = Math.max(1, Math.round(r.width));
    canvas.height = Math.max(1, Math.round(r.height));
    baseScale = Math.min(canvas.width, canvas.height) / worldW;
  }
  window.addEventListener("resize", resize);
  resize();
  const cam = { x: worldW / 2, y: worldH / 2, zoom: 1 };
  let followId = null;

  function canvasCoords(e) {
    const r = canvas.getBoundingClientRect();
    return [
      (e.clientX - r.left) * (canvas.width / r.width),
      (e.clientY - r.top) * (canvas.height / r.height),
    ];
  }
  function screenToWorld(mx, my) {
    const s = baseScale * cam.zoom;
    return { x: (mx - canvas.width / 2) / s + cam.x, y: (my - canvas.height / 2) / s + cam.y };
  }
  function resetView() {
    cam.x = worldW / 2;
    cam.y = worldH / 2;
    cam.zoom = 1;
    unfollow();
  }
  function unfollow() {
    followId = null;
  }
  function followById(id) {
    followId = id | 0;
    if (cam.zoom < 4) cam.zoom = 8;
    toast("👁 Следим за клеткой #" + followId + " — 2×клик, чтобы отпустить");
  }

  // --- controls ---
  let playing = true;
  let speed = 4;
  let brush = "observe";
  let predatorsAnnounced = false;

  const playBtn = $("play");
  playBtn.onclick = () => {
    playing = !playing;
    playBtn.textContent = playing ? "⏸ Пауза" : "▶ Играть";
    playBtn.classList.toggle("primary", playing);
  };
  $("reset").onclick = () => {
    sim.reset((Math.random() * 0xffffffff) >>> 0);
    popHist.length = 0;
    predatorsAnnounced = false;
    resetView();
    toast("🌍 Новый мир засеян");
  };
  $("speed").oninput = (e) => (speed = +e.target.value);
  $("brush").onchange = (e) => (brush = e.target.value);
  $("mut").oninput = (e) => sim.set_mutation_rate(+e.target.value);
  $("regrow").oninput = (e) => sim.set_field_regrow(+e.target.value);
  $("eat").oninput = (e) => sim.set_eat_rate(+e.target.value);
  $("habcost").oninput = (e) => sim.set_habitat_cost(+e.target.value);
  $("gear").onclick = () => $("settings").classList.toggle("hidden");
  $("drawer-toggle").onclick = () => {
    const collapsed = $("drawer").classList.toggle("collapsed");
    $("drawer-toggle").textContent = collapsed ? "Данные ▸" : "Данные ◂";
  };
  $("species").addEventListener("click", (e) => {
    const row = e.target.closest("[data-bkt]");
    if (!row) return;
    const b = +row.dataset.bkt;
    selectedSpecies = selectedSpecies === b ? null : b;
    updateSpecies();
  });
  $("records").addEventListener("click", (e) => {
    const row = e.target.closest("[data-follow]");
    if (row) followById(+row.dataset.follow);
  });

  const presetSel = $("preset");
  for (let id = 0; id < Sim.preset_count(); id++) {
    const o = document.createElement("option");
    o.value = id;
    o.textContent = Sim.preset_name(id);
    presetSel.appendChild(o);
  }
  presetSel.onchange = (e) => {
    const id = +e.target.value;
    sim.load_preset(id, (Math.random() * 0xffffffff) >>> 0);
    popHist.length = 0;
    predatorsAnnounced = false;
    resetView();
    toast("🌍 Пресет: " + Sim.preset_name(id));
  };

  function applyBrush(mx, my) {
    const w = screenToWorld(mx, my);
    const cx = Math.floor(w.x / cellW);
    const cy = Math.floor(w.y / cellH);
    if (brush === "food") sim.inject(cx, cy, 3, 900);
    else if (brush === "spawn") for (let k = 0; k < 6; k++) sim.spawn(cx, cy, 350);
    else if (brush === "kill") sim.kill(cx - 3, cy - 3, cx + 3, cy + 3);
  }

  // --- mouse: zoom / pan / follow / paint ---
  let dragging = false;
  let downX = 0;
  let downY = 0;
  let moved = false;
  let panStart = null;

  canvas.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      const [mx, my] = canvasCoords(e);
      const before = screenToWorld(mx, my);
      cam.zoom = Math.max(1, Math.min(40, cam.zoom * (e.deltaY < 0 ? 1.15 : 1 / 1.15)));
      const after = screenToWorld(mx, my);
      cam.x += before.x - after.x;
      cam.y += before.y - after.y;
    },
    { passive: false }
  );

  canvas.addEventListener("mousedown", (e) => {
    $("settings").classList.add("hidden");
    const [mx, my] = canvasCoords(e);
    dragging = true;
    moved = false;
    downX = mx;
    downY = my;
    panStart = { x: cam.x, y: cam.y };
    if (brush !== "observe") applyBrush(mx, my);
  });
  canvas.addEventListener("mousemove", (e) => {
    const [mx, my] = canvasCoords(e);
    if (dragging) {
      if (Math.abs(mx - downX) + Math.abs(my - downY) > 3) moved = true;
      if (brush === "observe") {
        const s = baseScale * cam.zoom;
        cam.x = panStart.x - (mx - downX) / s;
        cam.y = panStart.y - (my - downY) / s;
      } else {
        applyBrush(mx, my);
      }
    } else if (brush === "observe" && followId === null) {
      inspectHover(mx, my);
    }
  });
  window.addEventListener("mouseup", () => {
    if (dragging && brush === "observe" && !moved) {
      const w = screenToWorld(downX, downY);
      const n = sim.nearest(w.x, w.y);
      if (n.length) followById(n[10]);
    }
    dragging = false;
  });
  canvas.addEventListener("dblclick", resetView);
  canvas.addEventListener("contextmenu", (e) => e.preventDefault());

  // --- inspector ---
  const inspectBody = $("inspect-body");
  let lastInspect = 0;
  function cellCard(px, py, energy, age, size, metab, repro, r, g, b, id, carn, brain, habitat, diet, tag) {
    const pred = carn > 0.12 ? "🔴 хищник" : "🌿 травоядное";
    const env = habitat < 0.4 ? "🌊 вода" : habitat > 0.6 ? "⛰ суша" : "🏖 берег";
    const food = diet < 0.35 ? "🟢 еда A" : diet > 0.65 ? "🟠 еда B" : "🍽 всеядное";
    return `
      <div class="row" style="margin:0 0 8px">
        <span><span class="swatch" style="background:rgb(${r | 0},${g | 0},${b | 0})"></span> клетка #${id | 0}${tag}</span>
        <span class="mono" style="color:var(--muted2)">возраст ${age | 0}</span>
      </div>
      <div class="row mono" style="margin:4px 0"><span>рацион</span><b>${pred}</b></div>
      <div class="row mono" style="margin:4px 0"><span>питание (ген)</span><b>${food} ${diet.toFixed(2)}</b></div>
      <div class="row mono" style="margin:4px 0"><span>среда (ген)</span><b>${env} ${habitat.toFixed(2)}</b></div>
      <div class="row mono" style="margin:4px 0"><span>энергия</span><b>${energy | 0}</b></div>
      <div class="row mono" style="margin:4px 0"><span>ген «размер»</span><b>${size.toFixed(2)}</b></div>
      <div class="row mono" style="margin:4px 0"><span>ген «обмен»</span><b>${metab.toFixed(2)}</b></div>
      <div class="row mono" style="margin:4px 0"><span>ген «размножение»</span><b>${repro.toFixed(2)}</b></div>
      <div class="row mono" style="margin:4px 0"><span>мозг 🧠</span><b>${brain | 0}</b></div>`;
  }
  function inspectHover(mx, my) {
    const now = performance.now();
    if (now - lastInspect < 70) return;
    lastInspect = now;
    const w = screenToWorld(mx, my);
    const n = sim.nearest(w.x, w.y);
    if (!n.length) {
      inspectBody.innerHTML = '<div class="empty">Здесь пусто.</div>';
      return;
    }
    const [, , energy, age, size, metab, repro, r, g, b, id, carn, brain, habitat, diet] = n;
    inspectBody.innerHTML = cellCard(0, 0, energy, age, size, metab, repro, r, g, b, id, carn, brain, habitat, diet, "");
  }

  // --- population chart + trait bars ---
  const chart = $("chart");
  const cctx = chart.getContext("2d");
  const popHist = [];
  const POP_MAX = 272;
  function drawChart() {
    cctx.clearRect(0, 0, chart.width, chart.height);
    if (popHist.length < 2) return;
    const max = Math.max(...popHist, 1);
    cctx.beginPath();
    for (let i = 0; i < popHist.length; i++) {
      const x = (i / (POP_MAX - 1)) * chart.width;
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
    const [size, metab, repro, carn] = sim.avg_traits();
    $("a-size").textContent = size.toFixed(3);
    $("a-metab").textContent = metab.toFixed(3);
    $("a-repro").textContent = repro.toFixed(3);
    $("a-carn").textContent = carn.toFixed(3);
    bar("size", (size - 0.4) / 1.8);
    bar("metab", (metab - 0.3) / 1.7);
    bar("repro", (repro - 0.5) / 1.0);
    bar("carn", carn);
  }
  function updateHealth() {
    const carn = sim.frac_carnivore();
    $("a-troph").textContent = Math.round(carn * 100) + "%";
    bar("troph", carn);
    const div = sim.diversity();
    $("a-div").textContent = div.toFixed(2);
    bar("div", div / 4.16);
    const spd = sim.avg_speed();
    $("a-move").textContent = spd.toFixed(2);
    bar("move", spd / 2.5);
    const dc = sim.deaths_recent();
    $("deaths").innerHTML = `🍽 ${dc[0]} · ⏳ ${dc[1]} · 🔴 ${dc[3]} · ☠ ${dc[2]}`;
    const hh = sim.habitat_hist();
    $("habitat").innerHTML = `🌊 ${hh[0].toLocaleString()} · 🏖 ${hh[1].toLocaleString()} · ⛰ ${hh[2].toLocaleString()}`;
    const dh = sim.diet_hist();
    $("diet").innerHTML = `🟢 ${dh[0].toLocaleString()} · 🍽 ${dh[1].toLocaleString()} · 🟠 ${dh[2].toLocaleString()}`;
  }
  let selectedSpecies = null;
  function speciesDetail(s) {
    const env = s.hab < 0.4 ? "🌊 водные" : s.hab > 0.6 ? "⛰ сухопутные" : "🏖 береговые";
    const carnPct = Math.round(s.carn * 100);
    const diet = s.carn > 0.12 ? "🔴 хищники" : "🌿 травоядные";
    const food = s.diet < 0.35 ? "🟢 еда A" : s.diet > 0.65 ? "🟠 еда B" : "🍽 всеядные";
    return `<div class="detail">
      <div>в среднем <b>${env}</b> · ген «среда» ${s.hab.toFixed(2)}</div>
      <div>где живут: 🌊 <b>${s.water.toLocaleString()}</b> · 🏖 <b>${s.shore.toLocaleString()}</b> · ⛰ <b>${s.land.toLocaleString()}</b></div>
      <div>рацион: <b>${diet}</b> · хищность ${carnPct}%</div>
      <div>питание: <b>${food}</b> (ген ${s.diet.toFixed(2)})</div>
      <div>размер <b>${s.size.toFixed(2)}</b> · мозг 🧠<b>${s.brain}</b> · энергия ~<b>${(+s.energy).toLocaleString()}</b> · всего <b>${s.count.toLocaleString()}</b></div>
    </div>`;
  }
  function updateSpecies() {
    $("a-brain").textContent = sim.avg_brain_complexity().toFixed(1);
    let list;
    try {
      list = JSON.parse(sim.species_json());
    } catch {
      return;
    }
    $("species").innerHTML = list
      .map((s) => {
        const on = s.bkt === selectedSpecies ? " on" : "";
        const row =
          `<div class="row clk${on}" data-bkt="${s.bkt}" style="margin:0"><span><span class="swatch" style="width:12px;height:12px;background:rgb(${s.r},${s.g},${s.b})"></span> ${s.name}</span>` +
          `<span class="mono" style="color:var(--muted2)">${s.count.toLocaleString()} · 🔴${Math.round(s.carn * 100)}% · 🧠${s.brain}</span></div>`;
        return row + (s.bkt === selectedSpecies ? speciesDetail(s) : "");
      })
      .join("");
  }
  function updateRecords() {
    let list;
    try {
      list = JSON.parse(sim.records());
    } catch {
      return;
    }
    $("records").innerHTML = list
      .map(
        (r) =>
          `<div class="row clk" data-follow="${r.id}" style="margin:0"><span><span class="swatch" style="width:12px;height:12px;background:rgb(${r.r},${r.g},${r.b})"></span> ${r.cat}</span>` +
          `<span class="mono" style="color:var(--muted2)">${r.env} ${r.val} · #${r.id}</span></div>`
      )
      .join("");
  }

  // --- toasts ---
  const toastsEl = $("toasts");
  function toast(text) {
    const dv = document.createElement("div");
    dv.className = "toast";
    dv.textContent = text;
    toastsEl.appendChild(dv);
    setTimeout(() => dv.classList.add("fade"), 3400);
    setTimeout(() => dv.remove(), 4100);
  }
  let lastNarr = performance.now();
  let lastNarrPop = sim.population();
  function narrate() {
    const now = performance.now();
    if (now - lastNarr < 2500) return;
    const pop = sim.population();
    const carn = sim.avg_traits()[3];
    const prev = lastNarrPop || 1;
    const ratio = pop / prev;
    if (!predatorsAnnounced && carn > 0.03 && pop > 50) {
      toast("🦈 Сами собой появились хищники!");
      predatorsAnnounced = true;
    } else if (pop === 0 && lastNarrPop > 0) toast("💀 Мир вымер — жми «Новый мир» или подсыпь еды");
    else if (ratio > 1.35) toast(`🌱 Вспышка размножения (+${(pop - prev).toLocaleString()})`);
    else if (ratio < 0.7) toast(`💀 Массовое вымирание (−${(prev - pop).toLocaleString()})`);
    lastNarr = now;
    lastNarrPop = pop;
  }

  // --- render ---
  const tickEl = $("s-tick");
  const popEl = $("s-pop");
  let frame = 0;

  function draw() {
    // field + signal heatmap at grid resolution
    const f = sim.field();
    const fb = sim.field_b();
    const sg = sim.signal();
    const dt = sim.detritus();
    const tr = sim.terrain();
    const el = sim.elevation();
    const waterCut = sim.water_level() * 255;
    const d = fimg.data;
    for (let i = 0, j = 0; i < f.length; i++, j += 4) {
      const v = f[i]; // food A (green)
      const b2 = fb[i]; // food B (amber)
      const s = sg[i];
      const de = dt[i];
      if (el[i] < waterCut) {
        // underwater: shallows are teal, the deep is dark blue — a barrier you can read at a glance
        const depth = (waterCut - el[i]) / (waterCut || 1);
        d[j] = 12 + s * 0.1 + b2 * 0.16;
        d[j + 1] = 42 + v * 0.3 + b2 * 0.12 + (1 - depth) * 22 - depth * 12 + de * 0.2;
        d[j + 2] = 70 + depth * 120 + s * 0.55;
      } else {
        // dry land: food A reads green, food B reads amber; barren tan, detritus/signal on top
        const barren = 255 - tr[i];
        d[j] = 10 + barren * 0.14 + s * 0.12 + de * 0.85 + b2 * 0.5;
        d[j + 1] = 20 + tr[i] * 0.05 + v * 0.62 + de * 0.2 + b2 * 0.32;
        d[j + 2] = 18 + barren * 0.05 + s * 0.7;
      }
      d[j + 3] = 255;
    }
    fctx.putImageData(fimg, 0, 0);

    const W = canvas.width;
    const H = canvas.height;
    const s = baseScale * cam.zoom;
    const ox = W / 2 - cam.x * s;
    const oy = H / 2 - cam.y * s;

    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.fillStyle = "#050705";
    ctx.fillRect(0, 0, W, H);
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(fieldCanvas, ox, oy, worldW * s, worldH * s);

    // organisms
    const p = sim.positions();
    const c = sim.colors();
    const sz = sim.sizes();
    const cn = sim.carnivory();
    const closeUp = cam.zoom > 2.5;
    const vel = closeUp ? sim.velocities() : null;
    for (let i = 0, k = 0, m = 0; i < p.length; i += 2, k += 3, m++) {
      const x = ox + p[i] * s;
      const y = oy + p[i + 1] * s;
      if (x < -20 || x > W + 20 || y < -20 || y > H + 20) continue;
      const rad = (1.2 + sz[m] * 1.4) * s * 0.8;
      if (cn[m] > 30) {
        ctx.fillStyle = "rgba(255,60,60,0.85)";
        const rr = rad + (closeUp ? 2 : 1.2);
        ctx.fillRect(x - rr, y - rr, rr * 2, rr * 2);
      }
      ctx.fillStyle = `rgb(${c[k]},${c[k + 1]},${c[k + 2]})`;
      ctx.fillRect(x - rad, y - rad, rad * 2, rad * 2);
      if (vel) {
        ctx.strokeStyle = "rgba(230,240,255,0.55)";
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(x, y);
        ctx.lineTo(x + vel[i] * s * 5, y + vel[i + 1] * s * 5);
        ctx.stroke();
      }
    }

    // drifting bloom (food-patch) centres
    const bl = sim.blooms();
    ctx.strokeStyle = "rgba(255,235,150,0.22)";
    ctx.lineWidth = 1;
    for (let i = 0; i < bl.length; i += 2) {
      ctx.beginPath();
      ctx.arc(ox + bl[i] * s, oy + bl[i + 1] * s, 45 * s, 0, 6.283);
      ctx.stroke();
    }

    // follow highlight (the followed cell is kept at screen centre)
    if (followId !== null) {
      ctx.strokeStyle = "rgba(120,230,255,0.9)";
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(W / 2, H / 2, Math.max(14, 4 * s), 0, 6.283);
      ctx.stroke();
    }

    // night dimming (the day/night cycle, visible)
    const dl = sim.daylight();
    if (dl < 1) {
      ctx.fillStyle = `rgba(6,10,28,${(1 - dl) * 0.4})`;
      ctx.fillRect(0, 0, W, H);
    }
  }

  function updateFollow() {
    if (followId === null) return;
    const info = sim.by_id(followId);
    if (!info.length) {
      toast("☠ Клетка #" + followId + " погибла");
      followId = null;
      return;
    }
    const [px, py, energy, age, size, metab, repro, r, g, b, carn, brain, habitat, diet] = info;
    cam.x = px;
    cam.y = py;
    inspectBody.innerHTML = cellCard(
      px, py, energy, age, size, metab, repro, r, g, b, followId, carn, brain, habitat, diet,
      ' <span style="color:var(--accent)">· следим</span>'
    );
  }

  function loop() {
    if (playing) sim.tick(speed);
    updateFollow();
    draw();
    if ((frame & 3) === 0) {
      popHist.push(sim.population());
      if (popHist.length > POP_MAX) popHist.shift();
      drawChart();
    }
    if ((frame & 7) === 0) {
      tickEl.textContent = sim.tick_count().toLocaleString();
      popEl.textContent = sim.population().toLocaleString();
      const dl = sim.daylight();
      $("a-day").textContent = (dl > 0.5 ? "☀️ " : "🌙 ") + dl.toFixed(2);
      updateTraits();
      updateHealth();
      updateSpecies();
      updateRecords();
      narrate();
    }
    frame++;
    requestAnimationFrame(loop);
  }

  window.__evo = {
    sim,
    cam,
    step: (n) => {
      sim.tick(n);
      draw();
      updateTraits();
    },
  };

  toast("👋 Живой мир. Крути колесо, чтобы приблизиться, и кликни клетку — следить за ней.");
  loop();
}

main();
