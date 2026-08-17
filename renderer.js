/* =========================================================================
   OPEN MIAMI — WebGL renderer.

   The Rust/wasm engine owns the simulation and describes each frame as a
   flat Float32Array command stream (plus a \x1f-separated text arena),
   handed over once per frame through window.frameRender. This module owns
   the canvas and the GPU: it executes the stream with a single batched
   triangle pipeline.

   Command opcodes — mirror of `mod op` in src/graphics.rs. Keep in sync.
     0 CLEAR      r g b a
     1 RECT       x y w h  r g b a
     2 RECT_LINES x y w h thickness  r g b a
     3 CIRCLE     x y radius  r g b a
     4 LINE       x1 y1 x2 y2 thickness  r g b a
     5 ARC        x y radius a0 a1  r g b a          (filled pie slice)
     6 TEXT       textIdx x y size  r g b a          (left / baseline)
     7 SAVE
     8 RESTORE
     9 TRANSLATE  x y
    10 ROTATE     angle
    11 ROBOT      colorIdx poseIdx weaponIdx x y angle sizePx time
    12 SCALE      sx sy
    13 SHOGGOTH   x y sizePx heading reveal time

   Everything is drawn as vertex-colored, textured triangles in one
   interleaved dynamic buffer (a 1x1 white texture stands in for solid
   geometry), so a frame typically costs a handful of draw calls: the batch
   only breaks when the bound texture changes (solids -> robot atlas ->
   solids -> glyph atlas -> shoggoth atlas).

   Text: VT323 ("GameFont") glyphs are rasterized lazily into a glyph-atlas
   texture via a scratch 2D canvas, then drawn as quads like everything
   else.

   Robots: true live 3D->2D, every frame, at continuous animation time. Each
   ROBOT command reserves a tile in a per-frame scratch atlas and queues a
   robot-core render (pass 1: lit boxes -> small scene FBO; pass 2: edge-ink /
   posterize / pixelate, transparent background -> that atlas tile). The
   queued renders run inside this same GL context right before the batch that
   samples them is drawn, so a robot costs two tiny passes plus one textured
   quad, with no tile cache, no quantization and no CPU readback.

   Shoggoth (the boss): the same mechanism through shoggoth-core.js — a SHOGGOTH
   command reserves a bigger tile in its own scratch atlas and queues a live
   render of the mass / mask / tentacles at (heading, reveal, time); the tile is
   drawn as an axis-aligned quad through the transform stack (its facing is
   baked into the render itself, not a quad rotation).
   ========================================================================= */

import { createRobotPipeline } from "./robot-core.js";
import { createShoggothPipeline } from "./shoggoth-core.js";

const TEXT_SEP = "\u001f";

/* ---- robot tables (indices mirror src/graphics.rs draw_robot) ----------- */
const ROBOT_COLORS = ["coral", "red", "violet", "magenta"];
const ROBOT_POSES = ["idle", "walk", "shoot", "hit"];
const ROBOT_WEAPONS = ["fist", "pistol", "machinegun", "shotgun"];
const ROBOT_TILE = 128; // per-robot tile resolution (px) in the scratch atlas
const ROBOT_PX = 3; // robot-core pixelation block size at this tile size
const ROBOT_ATLAS_SIZE = 1024; // 8x8 = 64 robots per batch; more just flush early

/* ---- shoggoth (boss) scratch tiles ---- */
const SHOG_TILE = 384; // the boss is large (and drawn ~1:1 at the camera zoom)
const SHOG_PX = 4; // shoggoth-core pixelation block size at this tile size
const SHOG_ATLAS_SIZE = 768; // 2x2 = 4 bosses per batch (one is the norm)

/* ---- glyph atlas config ------------------------------------------------- */
const GLYPH_FS = 48; // rasterization font size; quads scale from this
const GLYPH_PAD = 2; // padding inside each glyph cell
const GLYPH_ATLAS_SIZE = 1024;

const VS = `
attribute vec2 aPos;
attribute vec2 aUv;
attribute vec4 aColor;
uniform vec2 uRes;
varying vec2 vUv;
varying vec4 vColor;
void main(){
  vUv = aUv;
  vColor = aColor;
  gl_Position = vec4(aPos.x / uRes.x * 2.0 - 1.0, 1.0 - aPos.y / uRes.y * 2.0, 0.0, 1.0);
}
`;

const FS = `
precision mediump float;
varying vec2 vUv;
varying vec4 vColor;
uniform sampler2D uTex;
void main(){
  gl_FragColor = texture2D(uTex, vUv) * vColor;
}
`;

export function initRenderer(canvas) {
  const gl = canvas.getContext("webgl", {
    alpha: false,
    antialias: true,
    premultipliedAlpha: true,
    preserveDrawingBuffer: false,
  });
  if (!gl) {
    throw new Error("WebGL is not available; the game cannot render.");
  }

  /* ---- program ---- */
  function compile(type, src) {
    const s = gl.createShader(type);
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
      throw new Error("Shader compile failed: " + gl.getShaderInfoLog(s));
    }
    return s;
  }
  const prog = gl.createProgram();
  gl.attachShader(prog, compile(gl.VERTEX_SHADER, VS));
  gl.attachShader(prog, compile(gl.FRAGMENT_SHADER, FS));
  gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
    throw new Error("Program link failed: " + gl.getProgramInfoLog(prog));
  }
  gl.useProgram(prog);
  const loc = {
    aPos: gl.getAttribLocation(prog, "aPos"),
    aUv: gl.getAttribLocation(prog, "aUv"),
    aColor: gl.getAttribLocation(prog, "aColor"),
    uRes: gl.getUniformLocation(prog, "uRes"),
    uTex: gl.getUniformLocation(prog, "uTex"),
  };

  gl.enable(gl.BLEND);
  gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
  gl.disable(gl.DEPTH_TEST);
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
  gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);

  /* ---- interleaved dynamic vertex buffer: x y u v r g b a ---- */
  const FLOATS_PER_VERT = 8;
  const MAX_VERTS = 65536;
  const verts = new Float32Array(MAX_VERTS * FLOATS_PER_VERT);
  let vCount = 0;
  const vbo = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
  gl.bufferData(gl.ARRAY_BUFFER, verts.byteLength, gl.DYNAMIC_DRAW);
  const STRIDE = FLOATS_PER_VERT * 4;
  gl.enableVertexAttribArray(loc.aPos);
  gl.vertexAttribPointer(loc.aPos, 2, gl.FLOAT, false, STRIDE, 0);
  gl.enableVertexAttribArray(loc.aUv);
  gl.vertexAttribPointer(loc.aUv, 2, gl.FLOAT, false, STRIDE, 8);
  gl.enableVertexAttribArray(loc.aColor);
  gl.vertexAttribPointer(loc.aColor, 4, gl.FLOAT, false, STRIDE, 16);

  /* ---- textures ---- */
  function makeTexture(size) {
    const t = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    if (size) {
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, size, size, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    }
    return t;
  }

  // 1x1 white: solid geometry samples this so one program draws everything.
  const whiteTex = makeTexture();
  gl.bindTexture(gl.TEXTURE_2D, whiteTex);
  gl.texImage2D(
    gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE,
    new Uint8Array([255, 255, 255, 255])
  );

  const glyphTex = makeTexture(GLYPH_ATLAS_SIZE);

  /* ---- robot scratch atlas: a render target the robot passes draw into ---- */
  // Tiles are handed out per frame in stream order and recycled after every
  // flush (once the quads that sample them have been drawn), so the atlas
  // only ever needs to hold the robots of one batch.
  const robotTex = makeTexture(ROBOT_ATLAS_SIZE);
  const robotCols = Math.floor(ROBOT_ATLAS_SIZE / ROBOT_TILE);
  const robotSlots = robotCols * robotCols;
  const robotFbo = gl.createFramebuffer();
  gl.bindFramebuffer(gl.FRAMEBUFFER, robotFbo);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, robotTex, 0);
  if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) {
    throw new Error("Robot atlas framebuffer is incomplete; the game cannot render.");
  }
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  const robotPipe = createRobotPipeline(gl, { rt: ROBOT_TILE });
  // Robots queued for the current batch: (colorIdx, poseIdx, weaponIdx, time)
  // per slot, rendered into their tiles by flush() right before the draw.
  const robotQueue = new Float32Array(robotSlots * 4);
  let robotUsed = 0;
  // Reused per render so the per-frame robot path never allocates.
  const robotOpts = {
    pose: "idle", color: "coral", weapon: "fist", time: 0,
    facingDeg: 0, px: ROBOT_PX, transparent: true,
  };
  const robotTarget = { fbo: robotFbo, x: 0, y: 0, w: ROBOT_TILE, h: ROBOT_TILE };

  /* ---- shoggoth scratch atlas: same scheme, bigger tiles, its own pipeline ---- */
  const shogTex = makeTexture(SHOG_ATLAS_SIZE);
  const shogCols = Math.floor(SHOG_ATLAS_SIZE / SHOG_TILE);
  const shogSlots = shogCols * shogCols;
  const shogFbo = gl.createFramebuffer();
  gl.bindFramebuffer(gl.FRAMEBUFFER, shogFbo);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, shogTex, 0);
  if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) {
    throw new Error("Shoggoth atlas framebuffer is incomplete; the game cannot render.");
  }
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  const shogPipe = createShoggothPipeline(gl, { rt: SHOG_TILE });
  // Bosses queued for the current batch: (heading, reveal, time) per slot.
  const shogQueue = new Float32Array(shogSlots * 3);
  let shogUsed = 0;
  const shogOpts = {
    reveal: 0, time: 0, heading: 0, wander: false, px: SHOG_PX, transparent: true,
  };
  const shogTarget = { fbo: shogFbo, x: 0, y: 0, w: SHOG_TILE, h: SHOG_TILE };

  // Re-establish everything the batched pipeline relies on. The robot passes
  // rebind program/buffers/attribs/framebuffer/viewport/blend/depth, so this
  // runs after them (and it is cheap enough to be defensive about it).
  function bindBatchState() {
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.useProgram(prog);
    gl.disable(gl.DEPTH_TEST);
    gl.disable(gl.CULL_FACE);
    gl.disable(gl.SCISSOR_TEST);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
    gl.enableVertexAttribArray(loc.aPos);
    gl.vertexAttribPointer(loc.aPos, 2, gl.FLOAT, false, STRIDE, 0);
    gl.enableVertexAttribArray(loc.aUv);
    gl.vertexAttribPointer(loc.aUv, 2, gl.FLOAT, false, STRIDE, 8);
    gl.enableVertexAttribArray(loc.aColor);
    gl.vertexAttribPointer(loc.aColor, 4, gl.FLOAT, false, STRIDE, 16);
    gl.activeTexture(gl.TEXTURE0);
  }

  // Run the queued robot / shoggoth renders into their atlas tiles. Leaves the
  // batch state rebound (and TEXTURE0 unbound — flush binds what it needs).
  function renderQueuedSprites() {
    // Our attrib arrays would otherwise stay enabled (pointing at the batch
    // VBO) while the sprite programs draw; keep the pipelines disjoint.
    gl.disableVertexAttribArray(loc.aPos);
    gl.disableVertexAttribArray(loc.aUv);
    gl.disableVertexAttribArray(loc.aColor);
    for (let i = 0; i < robotUsed; i++) {
      const q = i * 4;
      robotOpts.color = ROBOT_COLORS[robotQueue[q] | 0] || ROBOT_COLORS[0];
      robotOpts.pose = ROBOT_POSES[robotQueue[q + 1] | 0] || ROBOT_POSES[0];
      robotOpts.weapon = ROBOT_WEAPONS[robotQueue[q + 2] | 0] || ROBOT_WEAPONS[0];
      robotOpts.time = robotQueue[q + 3];
      robotTarget.x = (i % robotCols) * ROBOT_TILE;
      robotTarget.y = Math.floor(i / robotCols) * ROBOT_TILE;
      robotPipe.render(robotOpts, robotTarget);
    }
    for (let i = 0; i < shogUsed; i++) {
      const q = i * 3;
      shogOpts.heading = shogQueue[q];
      shogOpts.reveal = shogQueue[q + 1];
      shogOpts.time = shogQueue[q + 2];
      shogTarget.x = (i % shogCols) * SHOG_TILE;
      shogTarget.y = Math.floor(i / shogCols) * SHOG_TILE;
      shogPipe.render(shogOpts, shogTarget);
    }
    // The pipelines sampled their own scene texture on TEXTURE0; drop it so an
    // atlas is never both bound for sampling and attached to a framebuffer.
    gl.bindTexture(gl.TEXTURE_2D, null);
    bindBatchState();
  }

  // The pipeline's constructor left its own buffers bound: put ours back.
  bindBatchState();

  let boundTex = null;
  function flush() {
    // before the batch that samples them
    if (robotUsed > 0 || shogUsed > 0) renderQueuedSprites();
    if (vCount === 0) {
      robotUsed = 0;
      shogUsed = 0;
      return;
    }
    gl.bindTexture(gl.TEXTURE_2D, boundTex || whiteTex);
    gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, verts.subarray(0, vCount * FLOATS_PER_VERT));
    gl.drawArrays(gl.TRIANGLES, 0, vCount);
    vCount = 0;
    robotUsed = 0; // the quads sampling this batch's tiles are submitted: recycle
    shogUsed = 0;
  }

  function setTexture(tex) {
    if (boundTex !== tex) {
      flush();
      boundTex = tex;
    }
  }

  /* ---- transform stack (canvas-style: translate/rotate only) ---- */
  // Row form [a, b, c, d, e, f]: x' = a*x + c*y + e ; y' = b*x + d*y + f
  let m = [1, 0, 0, 1, 0, 0];
  const stack = [];
  function tSave() {
    stack.push(m.slice());
  }
  function tRestore() {
    if (stack.length) m = stack.pop();
  }
  function tTranslate(x, y) {
    m[4] += m[0] * x + m[2] * y;
    m[5] += m[1] * x + m[3] * y;
  }
  function tScale(sx, sy) {
    m[0] *= sx; m[1] *= sx;
    m[2] *= sy; m[3] *= sy;
  }
  function tRotate(angle) {
    const c = Math.cos(angle), s = Math.sin(angle);
    const a0 = m[0], b0 = m[1], c0 = m[2], d0 = m[3];
    m[0] = a0 * c + c0 * s;
    m[1] = b0 * c + d0 * s;
    m[2] = -a0 * s + c0 * c;
    m[3] = -b0 * s + d0 * c;
  }

  function vert(x, y, u, v, r, g, b, a) {
    if (vCount >= MAX_VERTS) flush(); // order-safe: same texture, same state
    const o = vCount * FLOATS_PER_VERT;
    verts[o] = m[0] * x + m[2] * y + m[4];
    verts[o + 1] = m[1] * x + m[3] * y + m[5];
    verts[o + 2] = u;
    verts[o + 3] = v;
    verts[o + 4] = r;
    verts[o + 5] = g;
    verts[o + 6] = b;
    verts[o + 7] = a;
    vCount++;
  }

  // Textured axis-aligned quad in *local* space (goes through the transform).
  function quad(x, y, w, h, u0, v0, u1, v1, r, g, b, a) {
    vert(x, y, u0, v0, r, g, b, a);
    vert(x + w, y, u1, v0, r, g, b, a);
    vert(x + w, y + h, u1, v1, r, g, b, a);
    vert(x, y, u0, v0, r, g, b, a);
    vert(x + w, y + h, u1, v1, r, g, b, a);
    vert(x, y + h, u0, v1, r, g, b, a);
  }

  function solidRect(x, y, w, h, r, g, b, a) {
    setTexture(whiteTex);
    quad(x, y, w, h, 0.5, 0.5, 0.5, 0.5, r, g, b, a);
  }

  // Stroke centered on the rect edges, matching canvas strokeRect.
  function rectLines(x, y, w, h, t, r, g, b, a) {
    const ht = t / 2;
    solidRect(x - ht, y - ht, w + t, t, r, g, b, a); // top
    solidRect(x - ht, y + h - ht, w + t, t, r, g, b, a); // bottom
    solidRect(x - ht, y + ht, t, h - t, r, g, b, a); // left
    solidRect(x + w - ht, y + ht, t, h - t, r, g, b, a); // right
  }

  function circle(x, y, radius, r, g, b, a) {
    setTexture(whiteTex);
    const segs = Math.max(12, Math.min(48, Math.ceil(radius)));
    for (let i = 0; i < segs; i++) {
      const a0 = (i / segs) * Math.PI * 2;
      const a1 = ((i + 1) / segs) * Math.PI * 2;
      vert(x, y, 0.5, 0.5, r, g, b, a);
      vert(x + Math.cos(a0) * radius, y + Math.sin(a0) * radius, 0.5, 0.5, r, g, b, a);
      vert(x + Math.cos(a1) * radius, y + Math.sin(a1) * radius, 0.5, 0.5, r, g, b, a);
    }
  }

  // Filled pie slice from a0 to a1 (canvas arc + close + fill semantics).
  function arcPie(x, y, radius, a0, a1, r, g, b, a) {
    setTexture(whiteTex);
    let span = a1 - a0;
    if (span < 0) span += Math.PI * 2;
    const segs = Math.max(4, Math.ceil((span / (Math.PI * 2)) * 48));
    for (let i = 0; i < segs; i++) {
      const s0 = a0 + (span * i) / segs;
      const s1 = a0 + (span * (i + 1)) / segs;
      vert(x, y, 0.5, 0.5, r, g, b, a);
      vert(x + Math.cos(s0) * radius, y + Math.sin(s0) * radius, 0.5, 0.5, r, g, b, a);
      vert(x + Math.cos(s1) * radius, y + Math.sin(s1) * radius, 0.5, 0.5, r, g, b, a);
    }
  }

  // Butt-capped line segment as a quad (canvas default lineCap).
  function line(x1, y1, x2, y2, t, r, g, b, a) {
    setTexture(whiteTex);
    const dx = x2 - x1, dy = y2 - y1;
    const len = Math.hypot(dx, dy);
    if (len < 1e-6) return;
    const nx = (-dy / len) * (t / 2);
    const ny = (dx / len) * (t / 2);
    vert(x1 + nx, y1 + ny, 0.5, 0.5, r, g, b, a);
    vert(x2 + nx, y2 + ny, 0.5, 0.5, r, g, b, a);
    vert(x2 - nx, y2 - ny, 0.5, 0.5, r, g, b, a);
    vert(x1 + nx, y1 + ny, 0.5, 0.5, r, g, b, a);
    vert(x2 - nx, y2 - ny, 0.5, 0.5, r, g, b, a);
    vert(x1 - nx, y1 - ny, 0.5, 0.5, r, g, b, a);
  }

  /* ---- glyph atlas: lazy VT323 rasterization ---- */
  const glyphs = new Map(); // char -> {u0,v0,u1,v1,w,h,advance}
  const glyphCellH = Math.ceil(GLYPH_FS * 1.3);
  const glyphBaseline = GLYPH_FS; // baseline offset from cell top
  let glyphPenX = 0;
  let glyphPenY = 0;
  const scratch = document.createElement("canvas");
  const scratchCtx = scratch.getContext("2d", { willReadFrequently: false });

  function bakeGlyph(ch) {
    scratchCtx.font = `${GLYPH_FS}px 'GameFont', monospace`;
    const advance = scratchCtx.measureText(ch).width;
    const cellW = Math.ceil(advance) + GLYPH_PAD * 2;
    if (glyphPenX + cellW > GLYPH_ATLAS_SIZE) {
      glyphPenX = 0;
      glyphPenY += glyphCellH;
    }
    if (glyphPenY + glyphCellH > GLYPH_ATLAS_SIZE) {
      // Atlas full (would need hundreds of distinct glyphs) — reset it.
      glyphs.clear();
      glyphPenX = 0;
      glyphPenY = 0;
    }
    scratch.width = cellW;
    scratch.height = glyphCellH;
    scratchCtx.clearRect(0, 0, cellW, glyphCellH);
    scratchCtx.font = `${GLYPH_FS}px 'GameFont', monospace`;
    scratchCtx.fillStyle = "#ffffff";
    scratchCtx.textBaseline = "alphabetic";
    scratchCtx.fillText(ch, GLYPH_PAD, glyphBaseline);
    flush(); // texture upload must not reorder past pending quads
    gl.bindTexture(gl.TEXTURE_2D, glyphTex);
    gl.texSubImage2D(gl.TEXTURE_2D, 0, glyphPenX, glyphPenY, gl.RGBA, gl.UNSIGNED_BYTE, scratch);
    const info = {
      u0: glyphPenX / GLYPH_ATLAS_SIZE,
      v0: glyphPenY / GLYPH_ATLAS_SIZE,
      u1: (glyphPenX + cellW) / GLYPH_ATLAS_SIZE,
      v1: (glyphPenY + glyphCellH) / GLYPH_ATLAS_SIZE,
      w: cellW,
      h: glyphCellH,
      advance,
    };
    glyphs.set(ch, info);
    glyphPenX += cellW;
    return info;
  }

  function drawText(text, x, y, size, r, g, b, a) {
    const s = size / GLYPH_FS;
    let pen = x;
    for (const ch of text) {
      if (ch === " ") {
        let info = glyphs.get(" ");
        if (!info) info = bakeGlyph(" ");
        pen += info.advance * s;
        continue;
      }
      let info = glyphs.get(ch);
      if (!info) info = bakeGlyph(ch);
      setTexture(glyphTex);
      quad(
        pen - GLYPH_PAD * s,
        y - glyphBaseline * s,
        info.w * s,
        info.h * s,
        info.u0, info.v0, info.u1, info.v1,
        r, g, b, a
      );
      pen += info.advance * s;
    }
  }

  /* ---- robots: queue a live render into a scratch tile, draw it as a quad ---- */
  // Facing is applied as quad rotation (the tile is rendered facing "up"), so
  // the robot goes through the transform stack like every other quad. `time`
  // is the engine's continuous animation clock, used as-is.
  function drawRobot(colorIdx, poseIdx, weaponIdx, x, y, angle, sizePx, time) {
    setTexture(robotTex);
    // Need a free tile AND room for the whole quad in this batch: a flush
    // recycles tiles, so the six verts of one robot must never straddle one.
    if (robotUsed >= robotSlots || vCount + 6 > MAX_VERTS) flush();
    const slot = robotUsed++;
    const q = slot * 4;
    robotQueue[q] = colorIdx;
    robotQueue[q + 1] = poseIdx;
    robotQueue[q + 2] = weaponIdx;
    robotQueue[q + 3] = time;
    const inset = 0.5; // half-texel inset against neighbor-tile bleed
    const tx = (slot % robotCols) * ROBOT_TILE;
    const ty = Math.floor(slot / robotCols) * ROBOT_TILE;
    // Pass 2 draws with GL's bottom-up viewport, so the tile's first row is
    // the robot's bottom: flip v so the quad reads it top-down like the canvas.
    const u0 = (tx + inset) / ROBOT_ATLAS_SIZE;
    const v0 = (ty + ROBOT_TILE - inset) / ROBOT_ATLAS_SIZE;
    const u1 = (tx + ROBOT_TILE - inset) / ROBOT_ATLAS_SIZE;
    const v1 = (ty + inset) / ROBOT_ATLAS_SIZE;
    const h = sizePx / 2;
    const c = Math.cos(angle), s = Math.sin(angle);
    // Rotated quad corners in local space (rotation about the robot's
    // center), then through the transform stack in vert().
    const ex = h * c, ey = h * s; // half-extent along the rotated x axis
    const fx = -h * s, fy = h * c; // half-extent along the rotated y axis
    const x0 = x - ex - fx, y0 = y - ey - fy; // top-left
    const x1 = x + ex - fx, y1 = y + ey - fy; // top-right
    const x2 = x + ex + fx, y2 = y + ey + fy; // bottom-right
    const x3 = x - ex + fx, y3 = y - ey + fy; // bottom-left
    vert(x0, y0, u0, v0, 1, 1, 1, 1);
    vert(x1, y1, u1, v0, 1, 1, 1, 1);
    vert(x2, y2, u1, v1, 1, 1, 1, 1);
    vert(x0, y0, u0, v0, 1, 1, 1, 1);
    vert(x2, y2, u1, v1, 1, 1, 1, 1);
    vert(x3, y3, u0, v1, 1, 1, 1, 1);
  }

  /* ---- shoggoth: queue a live boss render into a scratch tile, draw it as a quad ---- */
  // Axis-aligned quad of sizePx centered on (x, y), through the transform
  // stack. `heading` (radians, screen convention: 0 = +x, PI/2 = +y/down) is
  // what the mask leans toward; `reveal` 0..1 is the mask-off progress (0 =
  // masked, 1 = raw form); `time` is the engine's continuous clock.
  function drawShoggoth(x, y, sizePx, heading, reveal, time) {
    setTexture(shogTex);
    if (shogUsed >= shogSlots || vCount + 6 > MAX_VERTS) flush();
    const slot = shogUsed++;
    const q = slot * 3;
    shogQueue[q] = heading;
    shogQueue[q + 1] = reveal;
    shogQueue[q + 2] = time;
    const inset = 0.5;
    const tx = (slot % shogCols) * SHOG_TILE;
    const ty = Math.floor(slot / shogCols) * SHOG_TILE;
    // v flipped: pass 2 renders bottom-up (see drawRobot)
    const u0 = (tx + inset) / SHOG_ATLAS_SIZE;
    const v0 = (ty + SHOG_TILE - inset) / SHOG_ATLAS_SIZE;
    const u1 = (tx + SHOG_TILE - inset) / SHOG_ATLAS_SIZE;
    const v1 = (ty + inset) / SHOG_ATLAS_SIZE;
    const h = sizePx / 2;
    quad(x - h, y - h, sizePx, sizePx, u0, v0, u1, v1, 1, 1, 1, 1);
  }

  /* ---- frame execution ---- */
  function frameRender(cmds, textArena) {
    const w = canvas.width, h = canvas.height;
    gl.viewport(0, 0, w, h);
    gl.useProgram(prog);
    gl.uniform2f(loc.uRes, w, h);
    gl.uniform1i(loc.uTex, 0);
    gl.activeTexture(gl.TEXTURE0);

    const texts = textArena.length ? textArena.split(TEXT_SEP) : [];
    m = [1, 0, 0, 1, 0, 0];
    stack.length = 0;
    boundTex = null;
    vCount = 0;
    robotUsed = 0;
    shogUsed = 0;

    let i = 0;
    const n = cmds.length;
    while (i < n) {
      const op = cmds[i++];
      switch (op) {
        case 0: { // CLEAR
          flush();
          gl.clearColor(cmds[i], cmds[i + 1], cmds[i + 2], 1.0);
          gl.clear(gl.COLOR_BUFFER_BIT);
          i += 4;
          break;
        }
        case 1: // RECT
          solidRect(cmds[i], cmds[i + 1], cmds[i + 2], cmds[i + 3],
            cmds[i + 4], cmds[i + 5], cmds[i + 6], cmds[i + 7]);
          i += 8;
          break;
        case 2: // RECT_LINES
          rectLines(cmds[i], cmds[i + 1], cmds[i + 2], cmds[i + 3], cmds[i + 4],
            cmds[i + 5], cmds[i + 6], cmds[i + 7], cmds[i + 8]);
          i += 9;
          break;
        case 3: // CIRCLE
          circle(cmds[i], cmds[i + 1], cmds[i + 2],
            cmds[i + 3], cmds[i + 4], cmds[i + 5], cmds[i + 6]);
          i += 7;
          break;
        case 4: // LINE
          line(cmds[i], cmds[i + 1], cmds[i + 2], cmds[i + 3], cmds[i + 4],
            cmds[i + 5], cmds[i + 6], cmds[i + 7], cmds[i + 8]);
          i += 9;
          break;
        case 5: // ARC
          arcPie(cmds[i], cmds[i + 1], cmds[i + 2], cmds[i + 3], cmds[i + 4],
            cmds[i + 5], cmds[i + 6], cmds[i + 7], cmds[i + 8]);
          i += 9;
          break;
        case 6: { // TEXT
          const text = texts[cmds[i] | 0] ?? "";
          drawText(text, cmds[i + 1], cmds[i + 2], cmds[i + 3],
            cmds[i + 4], cmds[i + 5], cmds[i + 6], cmds[i + 7]);
          i += 8;
          break;
        }
        case 7: // SAVE
          tSave();
          break;
        case 8: // RESTORE
          tRestore();
          break;
        case 9: // TRANSLATE
          tTranslate(cmds[i], cmds[i + 1]);
          i += 2;
          break;
        case 10: // ROTATE
          tRotate(cmds[i]);
          i += 1;
          break;
        case 11: // ROBOT
          drawRobot(cmds[i], cmds[i + 1], cmds[i + 2], cmds[i + 3], cmds[i + 4],
            cmds[i + 5], cmds[i + 6], cmds[i + 7]);
          i += 8;
          break;
        case 12: // SCALE
          tScale(cmds[i], cmds[i + 1]);
          i += 2;
          break;
        case 13: // SHOGGOTH
          drawShoggoth(cmds[i], cmds[i + 1], cmds[i + 2], cmds[i + 3], cmds[i + 4],
            cmds[i + 5]);
          i += 6;
          break;
        default:
          // Unknown opcode: the stream is corrupt; stop rather than
          // misinterpret the remaining floats.
          console.error("frameRender: unknown opcode", op, "at", i - 1);
          i = n;
          break;
      }
    }
    flush();
  }

  return frameRender;
}
