// Canvas viewer over the deterministic sim-core (wasm). The unit of life is a multicellular
// organism: a body of cells grown from one genome. We render each body as a cohesive membraned
// blob of rounded, role-coloured cells — not squares.

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
  const $ = (id) => document.getElementById(id);

  const fieldCanvas = document.createElement("canvas");
  fieldCanvas.width = gridW;
  fieldCanvas.height = gridH;
  const fctx = fieldCanvas.getContext("2d");
  const fimg = fctx.createImageData(gridW, gridH);

  // --- camera ---
  let baseScale = 1;
  const cam = { x: worldW / 2, y: worldH / 2, zoom: 1 };
  function resize() {
    const r = canvas.getBoundingClientRect();
    canvas.width = Math.max(1, Math.round(r.width));
    canvas.height = Math.max(1, Math.round(r.height));
    baseScale = Math.min(canvas.width, canvas.height) / worldW;
  }
  window.addEventListener("resize", resize);
  resize();

  const scale = () => baseScale * cam.zoom;
  function screenToWorld(mx, my) {
    const s = scale();
    return { x: (mx - canvas.width / 2) / s + cam.x, y: (my - canvas.height / 2) / s + cam.y };
  }
  function mouseXY(e) {
    const r = canvas.getBoundingClientRect();
    return [
      (e.clientX - r.left) * (canvas.width / r.width),
      (e.clientY - r.top) * (canvas.height / r.height),
    ];
  }
  function resetView() {
    cam.x = worldW / 2;
    cam.y = worldH / 2;
    cam.zoom = 1;
  }

  // --- state ---
  let playing = true;
  let speed = 2;
  let brush = "observe";
  let colorBy = "role";
  let followId = null;

  // --- controls ---
  const playBtn = $("play");
  playBtn.onclick = () => {
    playing = !playing;
    playBtn.textContent = playing ? "⏸ Пауза" : "▶ Играть";
    playBtn.classList.toggle("primary", playing);
  };
  $("reset").onclick = () => {
    sim.reset((Math.random() * 0xffffffff) >>> 0);
    followId = null;
    resetView();
    toast("🌍 Новый мир");
  };
  $("speed").oninput = (e) => (speed = +e.target.value);
  $("brush").onchange = (e) => (brush = e.target.value);
  $("colorby").onchange = (e) => {
    colorBy = e.target.value;
    updateColorLegend();
  };
  $("gear").onclick = () => $("settings").classList.toggle("hidden");
  $("drawer-toggle").onclick = () => {
    const d = $("drawer");
    d.classList.toggle("collapsed");
    $("drawer-toggle").textContent = d.classList.contains("collapsed") ? "Данные ▸" : "Данные ◂";
  };
  $("mut").oninput = (e) => sim.set_mutation_rate(+e.target.value);
  $("regrow").oninput = (e) => sim.set_field_regrow(+e.target.value);
  $("eat").oninput = (e) => sim.set_eat_rate(+e.target.value);
  $("bite").oninput = (e) => sim.set_bite_amount(+e.target.value);
  $("bloomrate").oninput = (e) => sim.set_bloom_rate(+e.target.value);

  // presets
  const presetSel = $("preset");
  for (let i = 0; i < Sim.preset_count(); i++) {
    const o = document.createElement("option");
    o.value = i;
    o.textContent = Sim.preset_name(i);
    presetSel.appendChild(o);
  }
  presetSel.onchange = (e) => {
    sim.load_preset(+e.target.value, (Math.random() * 0xffffffff) >>> 0);
    followId = null;
    resetView();
    toast("🌍 Пресет: " + Sim.preset_name(+e.target.value));
  };

  function applyBrush(mx, my) {
    const w = screenToWorld(mx, my);
    const cx = Math.floor(w.x / cellW);
    const cy = Math.floor(w.y / cellH);
    if (brush === "food") sim.inject(cx, cy, 3, 900);
    else if (brush === "bloom") sim.bloom(cx, cy);
    else if (brush === "spawn") for (let k = 0; k < 4; k++) sim.spawn(cx, cy, 350);
    else if (brush === "kill") sim.kill(cx - 3, cy - 3, cx + 3, cy + 3);
  }

  // --- mouse ---
  let dragging = false,
    moved = false,
    lastX = 0,
    lastY = 0;
  canvas.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      const [mx, my] = mouseXY(e);
      const before = screenToWorld(mx, my);
      cam.zoom = Math.min(20, Math.max(0.5, cam.zoom * (e.deltaY < 0 ? 1.12 : 0.89)));
      const after = screenToWorld(mx, my);
      cam.x += before.x - after.x;
      cam.y += before.y - after.y;
    },
    { passive: false }
  );
  canvas.addEventListener("mousedown", (e) => {
    const [mx, my] = mouseXY(e);
    dragging = true;
    moved = false;
    lastX = mx;
    lastY = my;
    if (brush !== "observe") applyBrush(mx, my);
  });
  canvas.addEventListener("mousemove", (e) => {
    const [mx, my] = mouseXY(e);
    if (dragging) {
      const dx = mx - lastX,
        dy = my - lastY;
      if (Math.abs(dx) + Math.abs(dy) > 3) moved = true;
      if (brush === "observe") {
        cam.x -= dx / scale();
        cam.y -= dy / scale();
      } else applyBrush(mx, my);
      lastX = mx;
      lastY = my;
    } else if (brush === "observe" && followId === null) {
      inspectHover(mx, my);
    }
  });
  window.addEventListener("mouseup", () => {
    if (dragging && brush === "observe" && !moved) {
      const w = screenToWorld(lastX, lastY);
      const info = sim.nearest(w.x, w.y);
      if (info.length) {
        followId = info[2] | 0;
        toast("🔎 Следим за организмом #" + followId);
      }
    }
    dragging = false;
  });
  canvas.addEventListener("dblclick", () => {
    followId = null;
    resetView();
  });
  canvas.addEventListener("contextmenu", (e) => e.preventDefault());

  // --- toasts ---
  function toast(msg) {
    const t = document.createElement("div");
    t.className = "toast";
    t.textContent = msg;
    $("toasts").appendChild(t);
    setTimeout(() => t.classList.add("fade"), 1400);
    setTimeout(() => t.remove(), 2100);
  }

  // --- background field image ---
  function buildField() {
    const f = sim.field();
    const el = sim.elevation();
    const wl = sim.water_level() * 255;
    const d = fimg.data;
    for (let i = 0; i < f.length; i++) {
      const water = el[i] < wl;
      const fo = f[i];
      let r, g, b;
      if (water) {
        r = 18;
        g = 34;
        b = 58;
      } else {
        r = 12;
        g = 17;
        b = 12;
      }
      g = Math.min(255, g + fo * 0.5);
      r = Math.min(255, r + fo * 0.05);
      const k = i * 4;
      d[k] = r;
      d[k + 1] = g;
      d[k + 2] = b;
      d[k + 3] = 255;
    }
    fctx.putImageData(fimg, 0, 0);
  }

  // --- colour of a cell by role ---
  function roleColor(feed01, struct01) {
    // feeder -> green, structural -> slate-blue, low both -> muted grey-green
    const r = Math.round(70 + struct01 * 80);
    const g = Math.round(95 + feed01 * 140 - struct01 * 25);
    const b = Math.round(75 + struct01 * 110);
    return `rgb(${Math.min(255, r)},${Math.min(255, g)},${Math.min(255, b)})`;
  }

  function updateColorLegend() {
    const el = $("colorlegend");
    if (colorBy === "role") {
      el.innerHTML =
        '<span><span class="sw" style="background:rgb(90,235,110)"></span>зелёные — кормовые клетки</span>' +
        '<span><span class="sw" style="background:rgb(150,120,185)"></span>сланцевые — структурные/защитные</span>' +
        '<span><span class="sw" style="background:rgb(120,140,120)"></span>серо-зелёные — неспециализированные</span>';
    } else {
      el.innerHTML =
        '<span><span class="sw" style="background:linear-gradient(90deg,#e44,#4e4,#46f)"></span>цвет — гены рода организма</span>';
    }
  }

  // --- draw ---
  function draw() {
    const W = canvas.width,
      H = canvas.height;
    const s = scale();
    const ox = W / 2 - cam.x * s;
    const oy = H / 2 - cam.y * s;
    ctx.clearRect(0, 0, W, H);

    // background food/water field
    buildField();
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(fieldCanvas, ox, oy, worldW * s, worldH * s);
    ctx.imageSmoothingEnabled = true;

    // bodies
    const p = sim.cell_positions();
    const sz = sim.cell_sizes();
    const roles = colorBy === "role" ? sim.cell_roles() : null;
    const cols = colorBy === "lineage" ? sim.cell_colors() : null;
    const n = p.length / 2;

    // pass 1 — membrane: a soft disc under each cell; overlapping cells of a body merge into a blob
    ctx.fillStyle = "rgba(140,190,150,0.13)";
    for (let m = 0; m < n; m++) {
      const x = ox + p[m * 2] * s;
      const y = oy + p[m * 2 + 1] * s;
      if (x < -30 || x > W + 30 || y < -30 || y > H + 30) continue;
      const rad = (1.4 + sz[m] * 1.3) * s;
      ctx.beginPath();
      ctx.arc(x, y, rad * 1.35, 0, 6.283);
      ctx.fill();
    }

    // pass 2 — cells: rounded, coloured by role or lineage
    for (let m = 0; m < n; m++) {
      const x = ox + p[m * 2] * s;
      const y = oy + p[m * 2 + 1] * s;
      if (x < -30 || x > W + 30 || y < -30 || y > H + 30) continue;
      const rad = (1.4 + sz[m] * 1.3) * s;
      if (roles) ctx.fillStyle = roleColor(roles[m * 2] / 255, roles[m * 2 + 1] / 255);
      else ctx.fillStyle = `rgb(${cols[m * 3]},${cols[m * 3 + 1]},${cols[m * 3 + 2]})`;
      ctx.beginPath();
      ctx.arc(x, y, rad, 0, 6.283);
      ctx.fill();
    }

    // transient food-burst events
    const bl = sim.blooms();
    for (let i = 0; i < bl.length; i += 4) {
      const bx = ox + bl[i] * s,
        by = oy + bl[i + 1] * s,
        br = Math.max(2, bl[i + 2] * s),
        frac = bl[i + 3];
      const grad = ctx.createRadialGradient(bx, by, 0, bx, by, br);
      grad.addColorStop(0, `rgba(130,235,140,${0.1 + 0.24 * frac})`);
      grad.addColorStop(1, "rgba(130,235,140,0)");
      ctx.fillStyle = grad;
      ctx.beginPath();
      ctx.arc(bx, by, br, 0, 6.283);
      ctx.fill();
    }

    // follow highlight
    if (followId !== null) {
      const info = sim.by_id(followId);
      if (info.length) {
        cam.x = info[0];
        cam.y = info[1];
        ctx.strokeStyle = "rgba(120,230,255,0.9)";
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(W / 2, H / 2, Math.max(16, 8 * s), 0, 6.283);
        ctx.stroke();
      }
    }

    // night dimming
    const dl = sim.daylight();
    if (dl < 0.5) {
      ctx.fillStyle = `rgba(4,8,20,${(0.5 - dl) * 0.7})`;
      ctx.fillRect(0, 0, W, H);
    }
  }

  // --- stats ---
  const chart = $("chart");
  const cctx = chart.getContext("2d");
  const popHist = [];
  const POP_MAX = 300;
  function drawChart() {
    cctx.clearRect(0, 0, chart.width, chart.height);
    if (popHist.length < 2) return;
    const mx = Math.max(...popHist, 10);
    cctx.strokeStyle = "#6ee7a0";
    cctx.lineWidth = 1.5;
    cctx.beginPath();
    for (let i = 0; i < popHist.length; i++) {
      const x = (i / (POP_MAX - 1)) * chart.width;
      const y = chart.height - (popHist[i] / mx) * (chart.height - 4) - 2;
      i ? cctx.lineTo(x, y) : cctx.moveTo(x, y);
    }
    cctx.stroke();
  }

  function pct(el, bar, v, max) {
    $(el).textContent = v.toFixed(2);
    if (bar) $(bar).style.width = Math.min(100, (v / max) * 100) + "%";
  }

  let frame = 0;
  function updateStats() {
    const pop = sim.population();
    $("s-tick").textContent = sim.tick_count();
    $("s-pop").textContent = pop;
    $("a-day").textContent = sim.daylight() > 0.5 ? "☀ день" : "🌙 ночь";

    popHist.push(pop);
    if (popHist.length > POP_MAX) popHist.shift();
    drawChart();

    $("a-body").textContent = sim.avg_body().toFixed(1);
    $("a-maxbody").textContent = sim.max_body();
    pct("a-dol", "b-dol", sim.dol(), 1);
    const diff = sim.diff_frac();
    $("a-diff").textContent = (diff * 100).toFixed(0) + "%";
    $("b-diff").style.width = diff * 100 + "%";
    $("a-brain").textContent = sim.avg_brain().toFixed(1);
    $("a-regnet").textContent = sim.avg_regnet().toFixed(1);
    $("a-genes").textContent = sim.avg_genome_len().toFixed(1);
    $("a-div").textContent = sim.diversity().toFixed(2);
    $("a-move").textContent = sim.avg_speed().toFixed(2);

    const d = sim.deaths_recent();
    $("deaths").textContent = `голод ${d[0]} · старость ${d[1]} · стёрты ${d[2]}`;

    if (followId !== null) updateFollow();
  }

  // --- inspector ---
  function bodyCard(info, tag) {
    const [, , id, energy, age, ncells, dol, feed, strc, brain, regnet, genes] = info;
    return `
      <div class="row" style="margin:0 0 8px">
        <span>организм #${id | 0}${tag}</span>
        <span class="mono" style="color:var(--muted2)">возраст ${age | 0}</span>
      </div>
      <div class="row mono" style="margin:4px 0"><span>клеток в теле</span><b>🧫 ${ncells | 0}</b></div>
      <div class="row mono" style="margin:4px 0"><span>разделение труда</span><b>DOL ${(+dol).toFixed(2)}</b></div>
      <div class="row mono" style="margin:4px 0"><span>роли (сред.)</span><b>🟢 корм ${(feed * 100).toFixed(0)}% · 🟦 защита ${(strc * 100).toFixed(0)}%</b></div>
      <div class="row mono" style="margin:4px 0"><span>энергия (котёл)</span><b>${energy | 0}</b></div>
      <div class="row mono" style="margin:4px 0"><span>мозг 🧠</span><b>${brain | 0}</b></div>
      <div class="row mono" style="margin:4px 0"><span>программа развития</span><b>${regnet | 0}</b></div>
      <div class="row mono" style="margin:4px 0"><span>геном</span><b>${genes | 0} генов</b></div>`;
  }
  let lastInspect = 0;
  function inspectHover(mx, my) {
    const now = performance.now();
    if (now - lastInspect < 70) return;
    lastInspect = now;
    const w = screenToWorld(mx, my);
    const info = sim.nearest(w.x, w.y);
    $("inspect-body").innerHTML = info.length
      ? bodyCard(info, "")
      : '<div class="empty">Здесь пусто.</div>';
  }
  function updateFollow() {
    const info = sim.by_id(followId);
    if (!info.length) {
      toast("☠ Организм #" + followId + " погиб");
      followId = null;
      return;
    }
    $("inspect-body").innerHTML = bodyCard(
      info,
      ' <span style="color:var(--accent)">· следим</span>'
    );
  }

  // --- loop ---
  function loop() {
    if (playing) sim.tick(speed);
    draw();
    if (frame % 8 === 0) updateStats();
    frame++;
    requestAnimationFrame(loop);
  }

  window.__evo = { sim, cam, step: (nn) => { sim.tick(nn); draw(); updateStats(); } };
  updateColorLegend();
  toast("🧫 Живые организмы. Крути колесо, кликни тело — следить.");
  loop();
}

main();
