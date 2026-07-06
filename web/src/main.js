// Minimal Canvas prototype driving the deterministic sim-core (compiled to wasm).
// Deliberately rough — this is the "touch it" slice, not the Phase-1 MVP renderer.

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
  const sx = canvas.width / worldW;
  const sy = canvas.height / worldH;

  // Offscreen field buffer, scaled up to the canvas for a soft heatmap.
  const fieldCanvas = document.createElement("canvas");
  fieldCanvas.width = gridW;
  fieldCanvas.height = gridH;
  const fctx = fieldCanvas.getContext("2d");
  const fimg = fctx.createImageData(gridW, gridH);

  // --- controls ---------------------------------------------------------
  let playing = true;
  let speed = 4;
  let brush = "food";

  const $ = (id) => document.getElementById(id);
  const playBtn = $("play");
  playBtn.onclick = () => {
    playing = !playing;
    playBtn.textContent = playing ? "⏸ Pause" : "▶ Play";
    playBtn.classList.toggle("primary", playing);
  };
  $("reset").onclick = () => sim.reset((Math.random() * 0xffffffff) >>> 0);
  $("speed").oninput = (e) => (speed = +e.target.value);
  $("brush").onchange = (e) => (brush = e.target.value);
  $("mut").oninput = (e) => sim.set_mutation_rate(+e.target.value);
  $("regrow").oninput = (e) => sim.set_field_regrow(+e.target.value);
  $("eat").oninput = (e) => sim.set_eat_rate(+e.target.value);

  canvas.addEventListener("mousedown", (e) => applyBrush(e));
  canvas.addEventListener("mousemove", (e) => {
    if (e.buttons & 1) applyBrush(e);
  });
  canvas.addEventListener("contextmenu", (e) => e.preventDefault());

  function applyBrush(e) {
    const rect = canvas.getBoundingClientRect();
    const wx = ((e.clientX - rect.left) / rect.width) * worldW;
    const wy = ((e.clientY - rect.top) / rect.height) * worldH;
    const cx = Math.floor(wx / cellW);
    const cy = Math.floor(wy / cellH);
    if (brush === "food") sim.inject(cx, cy, 3, 900);
    else if (brush === "spawn") {
      for (let k = 0; k < 6; k++) sim.spawn(cx, cy, 200);
    } else if (brush === "kill") sim.kill(cx - 3, cy - 3, cx + 3, cy + 3);
  }

  // --- render loop ------------------------------------------------------
  const tickEl = $("s-tick");
  const popEl = $("s-pop");
  const hashEl = $("s-hash");
  let frame = 0;

  function draw() {
    // field heatmap
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

    // organisms
    const p = sim.positions();
    const c = sim.colors();
    for (let i = 0, k = 0; i < p.length; i += 2, k += 3) {
      ctx.fillStyle = `rgb(${c[k]},${c[k + 1]},${c[k + 2]})`;
      ctx.fillRect(p[i] * sx - 1.2, p[i + 1] * sy - 1.2, 2.4, 2.4);
    }

    if ((frame & 7) === 0) {
      tickEl.textContent = sim.tick_count().toLocaleString();
      popEl.textContent = sim.population().toLocaleString();
      hashEl.textContent = sim.state_hash();
    }
  }

  function loop() {
    if (playing) sim.tick(speed);
    draw();
    frame++;
    requestAnimationFrame(loop);
  }

  // Debug/verification hook (harmless): drive the sim from the console/headless preview,
  // since requestAnimationFrame is throttled when the tab isn't foregrounded.
  window.__evo = {
    sim,
    step: (n) => {
      sim.tick(n);
      draw();
    },
  };

  loop();
}

main();
