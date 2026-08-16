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

   Everything is drawn as vertex-colored, textured triangles in one
   interleaved dynamic buffer (a 1x1 white texture stands in for solid
   geometry), so a frame typically costs a handful of draw calls: the batch
   only breaks when the bound texture changes (solids -> robot atlas ->
   solids -> glyph atlas).

   Text: VT323 ("GameFont") glyphs are rasterized lazily into a glyph-atlas
   texture via a scratch 2D canvas, then drawn as quads like everything
   else.

   Robots: the robot-core 3D->2D pipeline renders (color, pose, weapon,
   animation-frame) tiles on demand — animation time is quantized to a few
   frames per pose — and the tiles are cached in a texture atlas, so the
   steady-state cost of a fully animated 3D robot is one textured quad.
   ========================================================================= */

import { bakeSprite } from "./proto/robot-core.js";

const TEXT_SEP = "\u001f";

/* ---- robot tile tables (indices mirror src/graphics.rs draw_robot) ------ */
const ROBOT_COLORS = ["coral", "red", "violet", "magenta"];
const ROBOT_WEAPONS = ["fist", "pistol", "machinegun", "shotgun"];
// Animation cycle length and baked frame count per pose. Periods match the
// periodic time-functions in robot-core's posePlan: walk phase is t*2pi
// (period 1s), idle breath sin(t*1.9), shoot recoil sin(t*10), hit flinch
// repeats every 1.3s. `wrap:false` clamps instead of looping (the hit flinch
// plays once and settles — the engine sends time-since-impact).
const ROBOT_POSES = [
  { name: "idle", period: (2 * Math.PI) / 1.9, frames: 8, wrap: true },
  { name: "walk", period: 1.0, frames: 8, wrap: true },
  { name: "shoot", period: (2 * Math.PI) / 10, frames: 6, wrap: true },
  { name: "hit", period: 1.3, frames: 10, wrap: false },
];
const ROBOT_TILE = 128; // baked tile resolution (px) in the robot atlas
const ROBOT_BAKE_PX = 3; // robot-core pixelation block size at this tile size

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
  const ROBOT_ATLAS_SIZE = Math.min(2048, gl.getParameter(gl.MAX_TEXTURE_SIZE));
  const robotTex = makeTexture(ROBOT_ATLAS_SIZE);
  const robotCols = Math.floor(ROBOT_ATLAS_SIZE / ROBOT_TILE);
  const robotSlots = robotCols * robotCols;

  let boundTex = null;
  function flush() {
    if (vCount === 0) return;
    if (boundTex) {
      gl.bindTexture(gl.TEXTURE_2D, boundTex);
    }
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, verts.subarray(0, vCount * FLOATS_PER_VERT));
    gl.drawArrays(gl.TRIANGLES, 0, vCount);
    vCount = 0;
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

  /* ---- robot tile cache: live robot-core bakes -> atlas texture ---- */
  const robotTiles = new Map(); // key -> slot index
  let robotNextSlot = 0;

  function robotTile(colorIdx, poseIdx, weaponIdx, time) {
    const pose = ROBOT_POSES[poseIdx | 0] || ROBOT_POSES[0];
    let frame;
    if (pose.wrap) {
      const p = ((time % pose.period) + pose.period) % pose.period;
      frame = Math.min(pose.frames - 1, Math.floor((p / pose.period) * pose.frames));
    } else {
      frame = Math.max(0, Math.min(pose.frames - 1, Math.floor((time / pose.period) * pose.frames)));
    }
    const color = ROBOT_COLORS[colorIdx | 0] || ROBOT_COLORS[0];
    const weapon = ROBOT_WEAPONS[weaponIdx | 0] || ROBOT_WEAPONS[0];
    const key = `${color}:${pose.name}:${weapon}:${frame}`;
    let slot = robotTiles.get(key);
    if (slot === undefined) {
      if (robotNextSlot >= robotSlots) {
        // Atlas full — drop everything and rebake on demand (rare).
        robotTiles.clear();
        robotNextSlot = 0;
      }
      slot = robotNextSlot++;
      const frameTime = ((frame + 0.5) / pose.frames) * pose.period;
      const tile = bakeSprite({
        pose: pose.name,
        color,
        weapon,
        time: frameTime,
        facingDeg: 0,
        size: ROBOT_TILE,
        px: ROBOT_BAKE_PX,
        transparent: true,
      });
      flush();
      gl.bindTexture(gl.TEXTURE_2D, robotTex);
      gl.texSubImage2D(
        gl.TEXTURE_2D, 0,
        (slot % robotCols) * ROBOT_TILE,
        Math.floor(slot / robotCols) * ROBOT_TILE,
        gl.RGBA, gl.UNSIGNED_BYTE, tile
      );
      robotTiles.set(key, slot);
    }
    return slot;
  }

  function drawRobot(colorIdx, poseIdx, weaponIdx, x, y, angle, sizePx, time) {
    const slot = robotTile(colorIdx, poseIdx, weaponIdx, time);
    const inset = 0.5; // half-texel inset against neighbor-tile bleed
    const u0 = ((slot % robotCols) * ROBOT_TILE + inset) / ROBOT_ATLAS_SIZE;
    const v0 = (Math.floor(slot / robotCols) * ROBOT_TILE + inset) / ROBOT_ATLAS_SIZE;
    const u1 = ((slot % robotCols) * ROBOT_TILE + ROBOT_TILE - inset) / ROBOT_ATLAS_SIZE;
    const v1 = (Math.floor(slot / robotCols) * ROBOT_TILE + ROBOT_TILE - inset) / ROBOT_ATLAS_SIZE;
    setTexture(robotTex);
    const h = sizePx / 2;
    const c = Math.cos(angle), s = Math.sin(angle);
    // Rotated quad corners in local space, then through the transform stack.
    const px = [-h, h, h, -h];
    const py = [-h, -h, h, h];
    const cx = [], cy = [];
    for (let i = 0; i < 4; i++) {
      cx.push(x + px[i] * c - py[i] * s);
      cy.push(y + px[i] * s + py[i] * c);
    }
    vert(cx[0], cy[0], u0, v0, 1, 1, 1, 1);
    vert(cx[1], cy[1], u1, v0, 1, 1, 1, 1);
    vert(cx[2], cy[2], u1, v1, 1, 1, 1, 1);
    vert(cx[0], cy[0], u0, v0, 1, 1, 1, 1);
    vert(cx[2], cy[2], u1, v1, 1, 1, 1, 1);
    vert(cx[3], cy[3], u0, v1, 1, 1, 1, 1);
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
