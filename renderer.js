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
    14 POSTFX     kind t r g b                        (full-screen post pass)

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

   POSTFX: when a frame's stream contains opcode 14 (found by a cheap pre-scan
   over the opcode table), the whole frame is rendered into an offscreen scene
   framebuffer instead of the canvas, then drawn through a full-screen post
   shader. The kinds are a menu of Hotline-Miami-flavoured looks:
     0 BLUR-OUT      growing multi-tap blur + dissolve toward the colour (the ending)
     1 SYNTHWAVE CRT scanlines, chromatic split, vignette, grain (the credits)
     2 VHS TAPE      tracking band, per-line jitter, chroma bleed, dropouts
     3 DRUNK SWAY    slow rotation/zoom breathing, wavy warp, ghosting, hue drift
     4 CRT TUBE      barrel distortion, aperture grille, hard scanlines, flicker
     5 ACID TRIP     radial hue cycling, oversaturation, posterize, liquid warp
     6 DATAMOSH      slice/block displacement glitch, channel swaps, noise blocks
     7 NEON BLOOM    bright-pass glow, shadow tint toward the colour
     8 PIXEL MOSAIC  chunky pixelation + dithered posterize
     9 TUNNEL RUSH   radial zoom blur toward the centre (adrenaline)
   All kinds share the args `kind t r g b` (t = 0..1 strength, rgb = the
   effect's colour where it uses one). Only the last POSTFX of a frame applies.
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

/* ---- opcode argument counts (mirror of the table above); used by the POSTFX
   pre-scan, which has to walk the stream without executing it ---- */
const OP_ARGS = [4, 8, 9, 7, 9, 9, 8, 0, 0, 2, 1, 8, 2, 6, 5];
const OP_POSTFX = 14;

const POST_VS = `
attribute vec2 aPos;
varying vec2 vUv;
void main(){
  vUv = aPos * 0.5 + 0.5;
  gl_Position = vec4(aPos, 0.0, 1.0);
}
`;

// Full-screen post pass. One shader, one uniform selecting the look — the
// kinds are all cheap single-pass tricks (a few extra taps at most), kept
// deliberately dependency-free. See the header table for the kind list.
const POST_FS = `
precision mediump float;
varying vec2 vUv;
uniform sampler2D uScene;
uniform vec2 uRes;
uniform float uKind;
uniform float uT;
uniform vec3 uColor;
uniform float uTime;

float hash(vec2 p) {
  return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

// Hue rotation: Rodrigues rotation of the rgb vector about the gray axis.
vec3 hueShift(vec3 color, float a) {
  const vec3 k = vec3(0.57735);
  float ca = cos(a);
  return color * ca + cross(k, color) * sin(a) + k * dot(k, color) * (1.0 - ca);
}

float luma(vec3 c) {
  return dot(c, vec3(0.299, 0.587, 0.114));
}

// Chromatic split sample: r/b pulled apart along +-off.
vec3 splitSample(vec2 uv, vec2 off) {
  return vec3(
    texture2D(uScene, uv + off).r,
    texture2D(uScene, uv).g,
    texture2D(uScene, uv - off).b
  );
}

void main(){
  vec2 uv = vUv;
  vec3 c;
  float t = clamp(uT, 0.0, 1.0);
  // Coarse, time-jittered noise cell (grain / dissolve dither).
  float n = hash(floor(uv * uRes / 3.0) + floor(uTime * 24.0) * 0.371);
  float scan = 0.5 + 0.5 * sin(uv.y * uRes.y * 3.14159);
  if (uKind < 0.5) {
    // ---- 0 BLUR-OUT: two rings of taps whose radius grows with t ----
    float radPx = t * t * 34.0 + t * 2.0;
    vec2 px = radPx / uRes;
    vec3 acc = texture2D(uScene, uv).rgb * 2.0;
    float wsum = 2.0;
    for (int i = 0; i < 8; i++) {
      float a = float(i) * 0.785398 + uTime * 0.7;
      vec2 d = vec2(cos(a), sin(a));
      acc += texture2D(uScene, uv + d * px).rgb;
      acc += texture2D(uScene, uv + d * px * 0.5).rgb * 1.5;
      wsum += 2.5;
    }
    c = acc / wsum;
    // Dissolve toward the colour, dithered by the grain so it eats in patches.
    float k = smoothstep(0.12, 1.0, t + (n - 0.5) * 0.35 * t);
    c = mix(c, uColor, k);
    c *= 1.0 - 0.22 * t * scan;
    c += (n - 0.5) * 0.12 * t;
  } else if (uKind < 1.5) {
    // ---- 1 SYNTHWAVE CRT: chromatic split, scanlines, vignette, grain ----
    c = splitSample(uv, vec2(1.6 * t / uRes.x, 0.0));
    c *= 1.0 - 0.28 * t * scan;
    vec2 q = uv * (1.0 - uv);
    float vig = pow(clamp(q.x * q.y * 18.0, 0.0, 1.0), 0.28 * t);
    c = c * vig + uColor * 0.10 * t * (1.0 - vig);
    c += (n - 0.5) * 0.06 * t;
  } else if (uKind < 2.5) {
    // ---- 2 VHS TAPE: tracking band, line jitter, chroma bleed, dropouts ----
    // A tracking band rolls up the screen; lines inside it tear hard.
    float yb = fract(uTime * 0.13);
    float db = abs(uv.y - yb);
    float band = smoothstep(0.045, 0.0, min(db, 1.0 - db));
    float ln = floor(uv.y * uRes.y);
    float jit = (hash(vec2(ln, floor(uTime * 24.0))) - 0.5)
      * (4.0 + band * 90.0) * t / uRes.x;
    vec2 suv = vec2(uv.x + jit + band * 0.02 * t * sin(uTime * 43.0 + uv.y * 61.0), uv.y);
    c = splitSample(suv, vec2(2.5 * t / uRes.x, 0.0));
    // Washed-out tape colour, whitened noise inside the band.
    c = mix(c, vec3(luma(c)), 0.25 * t);
    c += band * t * (0.18 + 0.45 * n);
    // Rare white dropout streaks.
    float drop = step(0.994, hash(vec2(ln, floor(uTime * 60.0) + 7.0)));
    c = mix(c, vec3(0.9), drop * 0.8 * t);
    // Head-switch noise bar pinned to the bottom edge.
    c = mix(c, vec3(n), step(0.972, uv.y) * 0.5 * t);
    c *= 1.0 - 0.18 * t * scan;
    c += (n - 0.5) * 0.10 * t;
  } else if (uKind < 3.5) {
    // ---- 3 DRUNK SWAY: rotation/zoom breathing, wavy warp, ghost, hue ----
    float asp = uRes.x / uRes.y;
    vec2 p = uv - 0.5;
    p.x *= asp;
    float ang = (sin(uTime * 0.8) * 0.045 + sin(uTime * 0.47 + 1.7) * 0.030) * t;
    float ca = cos(ang), sa = sin(ang);
    p = vec2(p.x * ca - p.y * sa, p.x * sa + p.y * ca);
    p /= 1.0 + (0.05 + 0.03 * sin(uTime * 1.1)) * t;
    p.x /= asp;
    vec2 wuv = p + 0.5;
    wuv += vec2(sin(wuv.y * 7.0 + uTime * 1.3), cos(wuv.x * 6.0 + uTime * 1.1)) * 0.006 * t;
    vec3 base = texture2D(uScene, wuv).rgb;
    // Double-vision ghost slowly orbiting the true image.
    vec2 gof = vec2(cos(uTime * 0.6), sin(uTime * 0.45)) * 9.0 * t / uRes;
    vec3 ghost = texture2D(uScene, wuv + gof).rgb;
    c = mix(base, max(base, ghost), 0.5 * t);
    c = hueShift(c, 0.5 * t * sin(uTime * 0.5));
    c *= 1.0 - 0.10 * t * scan;
    c += (n - 0.5) * 0.05 * t;
  } else if (uKind < 4.5) {
    // ---- 4 CRT TUBE: barrel distortion, aperture grille, flicker ----
    vec2 p = uv * 2.0 - 1.0;
    float r2 = dot(p, p);
    p *= 1.0 + 0.12 * t * r2;
    vec2 cuv = p * 0.5 + 0.5;
    // Off-tube pixels go black (the bezel).
    float inb = step(0.0, cuv.x) * step(cuv.x, 1.0) * step(0.0, cuv.y) * step(cuv.y, 1.0);
    c = splitSample(cuv, vec2(1.2 * t * (1.0 + r2) / uRes.x, 0.0));
    // Aperture grille: RGB phosphor triads across x.
    float px3 = mod(floor(cuv.x * uRes.x), 3.0);
    vec3 tri = vec3(step(px3, 0.5), step(0.5, px3) * step(px3, 1.5), step(1.5, px3));
    c *= mix(vec3(1.0), tri * 1.9 + 0.25, 0.7 * t);
    float scan2 = 0.5 + 0.5 * sin(cuv.y * uRes.y * 3.14159);
    c *= 1.0 - 0.35 * t * scan2;
    c *= 1.0 - 0.04 * t * (0.5 + 0.5 * sin(uTime * 87.0)); // mains flicker
    vec2 q = cuv * (1.0 - cuv);
    c *= pow(clamp(q.x * q.y * 25.0, 0.0, 1.0), 0.45 * t) * inb;
    c += (n - 0.5) * 0.05 * t * inb;
  } else if (uKind < 5.5) {
    // ---- 5 ACID TRIP: radial hue cycling, oversaturate, posterize ----
    vec2 wuv = uv + vec2(sin(uv.y * 12.0 + uTime * 1.7), cos(uv.x * 11.0 + uTime * 1.3)) * 0.004 * t;
    c = texture2D(uScene, wuv).rgb;
    float r = length(uv - 0.5);
    c = hueShift(c, t * (uTime * 1.2 + r * 6.0));
    c = mix(vec3(luma(c)), c, 1.0 + 0.9 * t); // oversaturate
    c = mix(c, floor(c * 6.0 + 0.5) / 6.0, 0.5 * t); // mild posterize
    c *= 1.0 - 0.10 * t * scan;
    c += (n - 0.5) * 0.05 * t;
  } else if (uKind < 6.5) {
    // ---- 6 DATAMOSH: slice/block displacement, channel swap, noise ----
    float rt = floor(uTime * 12.0);
    float seg = floor(uv.y * 28.0);
    float r1 = hash(vec2(seg, rt));
    float tear = step(0.72, r1);
    float shift = (r1 - 0.5) * 0.22 * t * tear;
    vec2 blk = floor(uv * vec2(12.0, 8.0));
    float br = hash(blk + rt * 0.13);
    shift += (hash(blk + rt) - 0.5) * 0.2 * t * step(0.93, br);
    vec2 guv = vec2(fract(uv.x + shift), uv.y);
    c = splitSample(guv, vec2((4.0 + 10.0 * tear) * t / uRes.x, 0.0));
    // Corrupted blocks: swapped channels or raw digital noise.
    c = mix(c, c.gbr, step(0.965, br) * t);
    vec3 noiseCol = vec3(hash(blk + rt * 3.7), hash(blk + rt * 5.1), hash(blk + rt * 7.3));
    c = mix(c, noiseCol, step(1.0 - 0.06 * t, hash(blk + rt + 31.0)));
    c *= 1.0 - 0.12 * t * scan;
    c += (n - 0.5) * 0.08 * t;
  } else if (uKind < 7.5) {
    // ---- 7 NEON BLOOM: bright-pass glow + shadow tint toward the colour ----
    c = texture2D(uScene, uv).rgb;
    vec3 glow = vec3(0.0);
    for (int i = 0; i < 8; i++) {
      float a = float(i) * 0.785398;
      vec2 d = vec2(cos(a), sin(a)) * (6.0 / uRes);
      glow += max(texture2D(uScene, uv + d).rgb - 0.45, 0.0);
      glow += max(texture2D(uScene, uv + d * 2.5).rgb - 0.45, 0.0) * 0.6;
    }
    glow /= 12.8;
    c += glow * 2.2 * t * (0.92 + 0.08 * sin(uTime * 9.0));
    c += uColor * 0.12 * t * (1.0 - luma(c)); // lift the shadows into neon
    c *= 1.0 - 0.10 * t * scan;
    c += (n - 0.5) * 0.04 * t;
  } else if (uKind < 8.5) {
    // ---- 8 PIXEL MOSAIC: chunky pixelation + dithered posterize ----
    float cell = 1.0 + 6.0 * t;
    vec2 id = floor(uv * uRes / cell);
    c = texture2D(uScene, (id + 0.5) * cell / uRes).rgb;
    float levels = 5.0;
    float dith = (hash(id) - 0.5) / levels;
    c = mix(c, floor((c + dith) * levels + 0.5) / levels, t);
    c *= 1.0 - 0.08 * t * scan;
  } else {
    // ---- 9 TUNNEL RUSH: radial zoom blur toward the centre ----
    vec2 p = uv - 0.5;
    vec3 acc = vec3(0.0);
    float wsum = 0.0;
    for (int i = 0; i < 10; i++) {
      float k = float(i) / 10.0;
      float w = 1.0 - k * 0.8;
      acc += texture2D(uScene, p * (1.0 - 0.22 * t * k) + 0.5).rgb * w;
      wsum += w;
    }
    c = acc / wsum;
    float rr = length(p);
    c *= 1.0 + 0.25 * t * (1.0 - smoothstep(0.0, 0.45, rr)); // hot centre
    vec2 q = uv * (1.0 - uv);
    c *= pow(clamp(q.x * q.y * 18.0, 0.0, 1.0), 0.4 * t);
    c += (n - 0.5) * 0.06 * t;
  }
  gl_FragColor = vec4(c, 1.0);
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

  /* ---- POSTFX: offscreen scene target + the full-screen post program ---- */
  const postProg = gl.createProgram();
  gl.attachShader(postProg, compile(gl.VERTEX_SHADER, POST_VS));
  gl.attachShader(postProg, compile(gl.FRAGMENT_SHADER, POST_FS));
  gl.linkProgram(postProg);
  if (!gl.getProgramParameter(postProg, gl.LINK_STATUS)) {
    throw new Error("Post program link failed: " + gl.getProgramInfoLog(postProg));
  }
  const postLoc = {
    aPos: gl.getAttribLocation(postProg, "aPos"),
    uScene: gl.getUniformLocation(postProg, "uScene"),
    uRes: gl.getUniformLocation(postProg, "uRes"),
    uKind: gl.getUniformLocation(postProg, "uKind"),
    uT: gl.getUniformLocation(postProg, "uT"),
    uColor: gl.getUniformLocation(postProg, "uColor"),
    uTime: gl.getUniformLocation(postProg, "uTime"),
  };
  const postVbo = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, postVbo);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, 1, 1, -1, -1, 1, 1, -1, 1]), gl.STATIC_DRAW);
  const sceneTex = makeTexture();
  const sceneFbo = gl.createFramebuffer();
  let sceneW = 0, sceneH = 0;
  // (Re)allocate the scene target to the canvas size (lazily, on first use /
  // resize) — the FBO is only touched on frames that carry a POSTFX.
  function ensureSceneTarget(w, h) {
    if (sceneW === w && sceneH === h) return;
    gl.bindTexture(gl.TEXTURE_2D, sceneTex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    gl.bindTexture(gl.TEXTURE_2D, null);
    gl.bindFramebuffer(gl.FRAMEBUFFER, sceneFbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, sceneTex, 0);
    if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) {
      throw new Error("Scene framebuffer is incomplete; the post pass cannot render.");
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    sceneW = w;
    sceneH = h;
  }
  // The framebuffer the batch draws into: null (the canvas) normally, the
  // scene FBO on frames that end in a post pass.
  let batchFbo = null;
  // The POSTFX request of the current frame (kind, t, r, g, b) or null.
  const postfx = { kind: 0, t: 0, r: 0, g: 0, b: 0 };
  let postfxActive = false;

  // Walk the stream by the opcode table (no execution) and pick up the LAST
  // POSTFX, if any — it must be known before the first draw so the whole
  // frame lands in the scene target.
  function scanPostfx(cmds) {
    let i = 0;
    const n = cmds.length;
    let found = false;
    while (i < n) {
      const op = cmds[i++];
      const args = OP_ARGS[op];
      if (args === undefined) break; // corrupt stream: frameRender reports it
      if (op === OP_POSTFX) {
        postfx.kind = cmds[i] | 0;
        postfx.t = cmds[i + 1];
        postfx.r = cmds[i + 2];
        postfx.g = cmds[i + 3];
        postfx.b = cmds[i + 4];
        found = true;
      }
      i += args;
    }
    return found;
  }

  // Draw the scene target to the canvas through the post shader, then hand
  // the GL state back to the batch pipeline.
  function runPostPass(w, h) {
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, w, h);
    gl.disable(gl.BLEND);
    gl.useProgram(postProg);
    gl.disableVertexAttribArray(loc.aUv);
    gl.disableVertexAttribArray(loc.aColor);
    gl.bindBuffer(gl.ARRAY_BUFFER, postVbo);
    gl.enableVertexAttribArray(postLoc.aPos);
    gl.vertexAttribPointer(postLoc.aPos, 2, gl.FLOAT, false, 0, 0);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, sceneTex);
    gl.uniform1i(postLoc.uScene, 0);
    gl.uniform2f(postLoc.uRes, w, h);
    gl.uniform1f(postLoc.uKind, postfx.kind);
    gl.uniform1f(postLoc.uT, postfx.t);
    gl.uniform3f(postLoc.uColor, postfx.r, postfx.g, postfx.b);
    gl.uniform1f(postLoc.uTime, (performance.now() % 100000) / 1000);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
    gl.bindTexture(gl.TEXTURE_2D, null);
    if (postLoc.aPos !== loc.aPos) gl.disableVertexAttribArray(postLoc.aPos);
    batchFbo = null;
    bindBatchState();
    gl.uniform2f(loc.uRes, w, h);
    gl.uniform1i(loc.uTex, 0);
  }

  // Re-establish everything the batched pipeline relies on. The robot passes
  // rebind program/buffers/attribs/framebuffer/viewport/blend/depth, so this
  // runs after them (and it is cheap enough to be defensive about it).
  function bindBatchState() {
    gl.bindFramebuffer(gl.FRAMEBUFFER, batchFbo);
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
    // A POSTFX anywhere in the frame routes the whole frame through the
    // offscreen scene target (decided up front, before the first draw).
    postfxActive = scanPostfx(cmds);
    if (postfxActive) ensureSceneTarget(w, h);
    batchFbo = postfxActive ? sceneFbo : null;
    gl.bindFramebuffer(gl.FRAMEBUFFER, batchFbo);
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
        case 14: // POSTFX (already picked up by the pre-scan)
          i += 5;
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
    if (postfxActive) runPostPass(w, h);
  }

  return frameRender;
}
