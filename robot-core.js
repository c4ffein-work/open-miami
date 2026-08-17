"use strict";
/* =========================================================================
   OPEN MIAMI - reusable 3D -> stylized 2D sprite renderer.
   Vanilla WebGL 1, no libraries. Shared by renderer.js (the game),
   tools/inspector.html and shoggoth-core.js (the boss, which builds on the
   same two-pass pipeline).

   Exports:
     PALETTES                       - color name -> {body,accent,trim}
     POSES                          - list of pose names
     WEAPONS                        - list of weapon names (fist/pistol/machinegun/shotgun)
     WEAPON_MODELS                  - name -> array of box parts (the 3D weapon models)
     M4                             - the pooled column-major mat4 helpers
     orbitVP / topDownVP            - camera builders (view-projection matrices)
     SpritePipeline                 - the pass-1 target + post pass base class that
                                      RobotPipeline (here) and ShoggothPipeline
                                      (shoggoth-core.js) extend: it owns the shared
                                      "inked" post-process, the FBO and the compile
                                      helpers, so no scene duplicates them
     createRobotPipeline(gl, {rt})  - the two-pass pipeline on an EXISTING WebGL
                                      context: .render(opts, target) draws one
                                      robot into a caller-provided framebuffer
                                      rect (or the canvas). This is what the game
                                      renderer uses to draw robots live, every
                                      frame, inside its own GL context.
     CanvasRenderer / makeBaker     - a pipeline bound to a canvas of its own /
                                      a "one frame -> fresh canvas" baker factory
                                      (both generic: shoggoth-core.js reuses them)
     createRenderer(canvas)         - a robot CanvasRenderer (owns a context +
                                      a pipeline); .render({...,weapon}) draws
                                      the held weapon
     bakeSprite({pose,color,facingDeg,px,time,size,weapon}) -> HTMLCanvasElement
                                      renders ONE baked top-down sprite frame.

   The robot is built from boxes + a tiny skeleton. A two-pass pipeline:
     pass 1: lit boxes -> offscreen RGBA texture (alpha carries a part id)
     pass 2: edge-detect + posterize + pixelate -> target ("inked" look)
   ========================================================================= */

/* ---------- palettes (player + 3 rogue palettes) ---------- */
export const PALETTES = {
  // body, accent(limbs/visor glow), dark trim
  coral:   {body:[0.98,0.52,0.42], accent:[1.0,0.75,0.55], trim:[0.35,0.14,0.12]},
  red:     {body:[0.86,0.16,0.18], accent:[1.0,0.45,0.30], trim:[0.28,0.05,0.06]},
  magenta: {body:[0.86,0.18,0.72], accent:[1.0,0.45,0.95], trim:[0.30,0.06,0.24]},
  violet:  {body:[0.55,0.35,0.90], accent:[0.75,0.60,1.0], trim:[0.18,0.12,0.34]},
};

export const POSES = ["idle", "walk", "shoot", "hit"];

/* ---------- weapons ----------
   Box-built weapon models in the same style as the robot. Each model is a small
   list of boxes expressed in the GUN-HAND's local frame (the right forearm's
   "elbow" node), pre-anchored at the grip. In that frame -Y points down the arm,
   which — once the arm is rotated forward to aim — becomes world +Z (forward) and
   slightly down: exactly the direction the shoot-pose barrel already sticks out.
   So a weapon whose body extends toward -Y reads as "held out in front" from the
   straight-down top-down bake.

   Each box: {t:[x,y,z], s:[x,y,z], c:[r,g,b] | "accent", id}
     - "accent" pulls the palette accent (muzzle/detail glow) so weapons tint
       with the character; solid arrays are fixed gunmetal.
   "fist" is the bare-hand / no-weapon case: no boxes, arm stays in its pose. */
export const WEAPONS = ["fist", "pistol", "machinegun", "shotgun"];

// Lightened for the true straight-down bake: dark gunmetal vanished against the
// near-black floor, so weapons now read as a clear steel silhouette from above.
const GUN_METAL = [0.44, 0.46, 0.52]; // body
const GUN_DARK  = [0.24, 0.25, 0.30]; // grip / mag / dark parts

export const WEAPON_MODELS = {
  fist: [],
  pistol: [
    {t:[0,-0.05, 0.04], s:[0.19,0.34,0.17], c:GUN_METAL, id:0.90}, // slide / body
    {t:[0,-0.26, 0.04], s:[0.10,0.15,0.11], c:"accent",  id:0.98}, // muzzle glow
    {t:[0, 0.11,-0.12], s:[0.12,0.22,0.16], c:GUN_DARK,  id:0.88}, // grip (hangs down/back)
  ],
  machinegun: [
    {t:[0,-0.12, 0.04], s:[0.21,0.64,0.18], c:GUN_METAL, id:0.90}, // long receiver + barrel
    {t:[0,-0.49, 0.04], s:[0.09,0.16,0.10], c:"accent",  id:0.98}, // muzzle glow
    {t:[0, 0.05,-0.16], s:[0.14,0.26,0.21], c:GUN_DARK,  id:0.87}, // magazine (hangs down)
    {t:[0, 0.13,-0.09], s:[0.11,0.18,0.15], c:GUN_DARK,  id:0.88}, // grip
    {t:[0,-0.30, 0.15], s:[0.09,0.42,0.06], c:GUN_DARK,  id:0.86}, // top rail
  ],
  shotgun: [
    {t:[0,-0.16, 0.06], s:[0.22,0.60,0.15], c:GUN_METAL, id:0.90}, // upper barrel
    {t:[0,-0.12,-0.07], s:[0.22,0.44,0.14], c:GUN_DARK,  id:0.89}, // pump / forestock
    {t:[0,-0.44, 0.06], s:[0.12,0.16,0.12], c:"accent",  id:0.98}, // muzzle glow
    {t:[0, 0.12,-0.11], s:[0.12,0.24,0.17], c:GUN_DARK,  id:0.88}, // grip / stock
  ],
};

/* ---------- tiny mat4 math (column-major, like GL) ----------
   Every matrix comes out of a bump-allocated scratch pool that is reset at the
   start of each render(): a robot is ~300 short-lived matrices, and recycling
   them keeps the per-frame path allocation-free once the pool has warmed up.
   Matrices are only valid until the next M4.reset() — nothing holds them. */
const _m4pool = [];
let _m4used = 0;
function m4alloc(){
  if(_m4used < _m4pool.length) return _m4pool[_m4used++];
  const m = new Float32Array(16);
  _m4pool.push(m); _m4used++;
  return m;
}
const _n3scratch = new Float32Array(9);
export const M4 = {
  reset(){ _m4used = 0; },
  ident(){const m=m4alloc();m.fill(0);m[0]=m[5]=m[10]=m[15]=1;return m;},
  mul(a,b){ // a*b
    const o=m4alloc();
    for(let r=0;r<4;r++)for(let c=0;c<4;c++){
      o[c*4+r]=a[0*4+r]*b[c*4+0]+a[1*4+r]*b[c*4+1]+a[2*4+r]*b[c*4+2]+a[3*4+r]*b[c*4+3];
    }
    return o;
  },
  translate(x,y,z){const m=M4.ident();m[12]=x;m[13]=y;m[14]=z;return m;},
  scale(x,y,z){const m=M4.ident();m[0]=x;m[5]=y;m[10]=z;return m;},
  rotX(a){const c=Math.cos(a),s=Math.sin(a);const m=M4.ident();m[5]=c;m[6]=s;m[9]=-s;m[10]=c;return m;},
  rotY(a){const c=Math.cos(a),s=Math.sin(a);const m=M4.ident();m[0]=c;m[2]=-s;m[8]=s;m[10]=c;return m;},
  rotZ(a){const c=Math.cos(a),s=Math.sin(a);const m=M4.ident();m[0]=c;m[1]=s;m[4]=-s;m[5]=c;return m;},
  ortho(l,r,b,t,n,f){
    const m=M4.ident();
    m[0]=2/(r-l);m[5]=2/(t-b);m[10]=-2/(f-n);
    m[12]=-(r+l)/(r-l);m[13]=-(t+b)/(t-b);m[14]=-(f+n)/(f-n);
    return m;
  },
  // look from eye toward center, up. returns view matrix.
  lookAt(eye,center,up){
    const z=norm(sub(eye,center));
    const x=norm(cross(up,z));
    const y=cross(z,x);
    const m=M4.ident();
    m[0]=x[0];m[4]=x[1];m[8]=x[2];
    m[1]=y[0];m[5]=y[1];m[9]=y[2];
    m[2]=z[0];m[6]=z[1];m[10]=z[2];
    m[12]=-dot(x,eye);m[13]=-dot(y,eye);m[14]=-dot(z,eye);
    return m;
  },
  // 3x3 normal matrix (upper-left 3x3; fine for our rotations + mild scales).
  // Returns a shared scratch: consume it (uniformMatrix3fv) before the next call.
  normalFromModel(m){
    const o=_n3scratch;
    o[0]=m[0];o[1]=m[1];o[2]=m[2]; o[3]=m[4];o[4]=m[5];o[5]=m[6]; o[6]=m[8];o[7]=m[9];o[8]=m[10];
    return o;
  }
};
function sub(a,b){return [a[0]-b[0],a[1]-b[1],a[2]-b[2]];}
function cross(a,b){return [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]];}
function dot(a,b){return a[0]*b[0]+a[1]*b[1]+a[2]*b[2];}
function norm(a){const l=Math.hypot(a[0],a[1],a[2])||1;return [a[0]/l,a[1]/l,a[2]/l];}

/* ---------- shaders ---------- */
const sceneVS = `
attribute vec3 aPos;
attribute vec3 aNormal;
uniform mat4 uMVP;
uniform mat3 uNormalMat;
varying vec3 vN;
varying float vDepth;
void main(){
  vec4 p = uMVP * vec4(aPos,1.0);
  gl_Position = p;
  vN = normalize(uNormalMat * aNormal);
  vDepth = p.z;
}
`;
const sceneFS = `
precision mediump float;
varying vec3 vN;
varying float vDepth;
uniform vec3 uColor;
uniform vec3 uAccent;
uniform float uId;
void main(){
  vec3 L = normalize(vec3(0.35, 0.9, 0.45));
  float ndl = max(dot(normalize(vN), L), 0.0);
  float amb = 0.35;
  float shade = amb + ndl*0.75;
  vec3 base = mix(uColor, uAccent, clamp(vN.y*0.5+0.2,0.0,1.0)*0.4);
  vec3 col = base * shade;
  // store part id in alpha so post pass can detect part boundaries as edges
  gl_FragColor = vec4(col, uId);
}
`;
const postVS = `
attribute vec2 aPos;
varying vec2 vUv;
void main(){ vUv = aPos*0.5+0.5; gl_Position = vec4(aPos,0.0,1.0); }
`;
const postFS = `
precision mediump float;
varying vec2 vUv;
uniform sampler2D uTex;
uniform vec2 uTexel;   // 1/size
uniform float uPx;     // pixel block size in px
uniform vec2 uSize;    // texture size
uniform float uTransparent; // 1.0 -> background blocks output alpha 0
uniform float uEdge;   // luma-gradient threshold that inks an edge
float luma(vec3 c){ return dot(c, vec3(0.299,0.587,0.114)); }
vec4 samp(vec2 uv){ return texture2D(uTex, uv); }
void main(){
  vec2 px = uSize;
  vec2 block = floor(vUv*px/uPx)*uPx + uPx*0.5;
  vec2 uv = block/px;

  vec4 c = samp(uv);
  vec3 col = c.rgb;

  vec2 t = uTexel*uPx;
  float l00=luma(samp(uv+vec2(-t.x,-t.y)).rgb);
  float l10=luma(samp(uv+vec2( 0.0,-t.y)).rgb);
  float l20=luma(samp(uv+vec2( t.x,-t.y)).rgb);
  float l01=luma(samp(uv+vec2(-t.x, 0.0)).rgb);
  float l21=luma(samp(uv+vec2( t.x, 0.0)).rgb);
  float l02=luma(samp(uv+vec2(-t.x, t.y)).rgb);
  float l12=luma(samp(uv+vec2( 0.0, t.y)).rgb);
  float l22=luma(samp(uv+vec2( t.x, t.y)).rgb);
  float gx = -l00 -2.0*l01 -l02 + l20 + 2.0*l21 + l22;
  float gy = -l00 -2.0*l10 -l20 + l02 + 2.0*l12 + l22;
  float lumEdge = sqrt(gx*gx+gy*gy);

  float a = c.a;
  float ai = max(max(abs(a-samp(uv+vec2(t.x,0.0)).a),abs(a-samp(uv+vec2(-t.x,0.0)).a)),
                 max(abs(a-samp(uv+vec2(0.0,t.y)).a),abs(a-samp(uv+vec2(0.0,-t.y)).a)));

  float silh = 0.0;
  if(a < 0.02){
    float near = max(max(samp(uv+vec2(t.x,0.0)).a,samp(uv+vec2(-t.x,0.0)).a),
                     max(samp(uv+vec2(0.0,t.y)).a,samp(uv+vec2(0.0,-t.y)).a));
    silh = near>0.02 ? 1.0 : 0.0;
  }

  float edge = max(max(step(uEdge, lumEdge), step(0.03, ai)), silh);

  float levels = 4.0;
  col = floor(col*levels + 0.5)/levels;

  if(a < 0.02 && silh < 0.5){
    if(uTransparent > 0.5){ gl_FragColor = vec4(0.0); return; }
    col = vec3(0.055,0.07,0.09);
  }

  col = mix(col, vec3(0.02,0.02,0.03), edge);

  gl_FragColor = vec4(col,1.0);
}
`;

/* ---------- unit cube geometry (positions + normals), centered, size 1 ---------- */
function makeCube(){
  const p=[], n=[];
  const faces=[
    {n:[0,0,1],  v:[[-.5,-.5,.5],[.5,-.5,.5],[.5,.5,.5],[-.5,.5,.5]]},
    {n:[0,0,-1], v:[[.5,-.5,-.5],[-.5,-.5,-.5],[-.5,.5,-.5],[.5,.5,-.5]]},
    {n:[1,0,0],  v:[[.5,-.5,.5],[.5,-.5,-.5],[.5,.5,-.5],[.5,.5,.5]]},
    {n:[-1,0,0], v:[[-.5,-.5,-.5],[-.5,-.5,.5],[-.5,.5,.5],[-.5,.5,-.5]]},
    {n:[0,1,0],  v:[[-.5,.5,.5],[.5,.5,.5],[.5,.5,-.5],[-.5,.5,-.5]]},
    {n:[0,-1,0], v:[[-.5,-.5,-.5],[.5,-.5,-.5],[.5,-.5,.5],[-.5,-.5,.5]]},
  ];
  for(const f of faces){
    const [a,b,c,d]=f.v;
    for(const tri of [[a,b,c],[a,c,d]]) for(const vtx of tri){ p.push(...vtx); n.push(...f.n); }
  }
  return {pos:new Float32Array(p), nrm:new Float32Array(n), count:p.length/3};
}

/* ---------- camera builders ---------- */
// TRUE straight-down top-down — the eye is directly over the character (no tilt),
// so a facing rotation is just the same sprite spun in-plane (identical from every
// direction, one bake per pose). This is exactly what the in-game camera sees.
export function topDownVP(halfV){
  halfV = halfV || 2.05;
  const proj = M4.ortho(-halfV,halfV,-halfV,halfV,0.1,40);
  const eye=[0, 9, 0], center=[0,0.9,0], up=[0,0,-1];
  return M4.mul(proj, M4.lookAt(eye,center,up));
}
// free orbit: yaw + pitch around the character, ortho so scale is stable.
export function orbitVP(yaw, pitch, halfV, center){
  halfV = halfV || 2.35;
  center = center || [0,0.95,0];
  const dist=12;
  pitch = Math.max(-1.45, Math.min(1.45, pitch));
  const cp=Math.cos(pitch), sp=Math.sin(pitch);
  const cy=Math.cos(yaw),   sy=Math.sin(yaw);
  const eye=[center[0]+dist*cp*sy, center[1]+dist*sp, center[2]+dist*cp*cy];
  // keep up stable; near-vertical is clamped above so lookAt won't degenerate.
  const up=[0,1,0];
  const proj = M4.ortho(-halfV,halfV,-halfV,halfV,0.1,60);
  return M4.mul(proj, M4.lookAt(eye,center,up));
}

/* ---------- pose -> skeleton drive ---------- */
// Returns the per-frame joint angles / offsets for a pose at a given time.
function posePlan(pose, time){
  const walkPhase = time*2.0*Math.PI;
  const swing  = Math.sin(walkPhase)*0.6;
  const swing2 = Math.sin(walkPhase+Math.PI)*0.6;

  // defaults (a neutral standing rig)
  const P = {
    bob:0, lean:0, zback:0,
    legA:0, legB:0,
    armLp:0.05, armRp:0.05, shoot:false,
    armRaise:0,          // extra shoulder-raise for both arms (defensive/idle)
    recoil:0,
  };

  switch(pose){
    case "walk":
      P.bob = Math.abs(Math.sin(walkPhase))*0.08;
      P.legA = swing;  P.legB = swing2;
      P.armLp = swing2; P.armRp = swing;   // arms counter-swing to legs
      break;

    case "shoot":
      P.shoot = true;
      P.legA = 0.12; P.legB = -0.12;
      P.armLp = 0.5;                        // support arm braces forward-ish
      P.recoil = Math.max(0.0, Math.sin(time*10.0))*0.18;
      break;

    case "idle": {
      const breath = Math.sin(time*1.9);
      P.bob   = breath*0.045;               // gentle chest/torso bob
      P.legA  = 0.015; P.legB = -0.015;     // weight shift, feet planted
      P.armLp = 0.08 + breath*0.05;         // arms sway slightly out of phase
      P.armRp = 0.08 - breath*0.05;
      break;
    }

    case "hit": {
      // periodic flinch: a sharp recoil back that decays, then repeats.
      const period = 1.3;
      const p = (time % period) / period;   // 0..1
      const env = Math.exp(-p*7.0);         // spike at impact, quick decay
      P.lean  = 0.55*env;                   // whole body rocks backward
      P.zback = -0.28*env;                  // and shoves back off its feet
      P.bob   = -0.05*env;
      P.legA  = -0.25*env; P.legB = 0.18*env;
      P.armRaise = 0.9*env;                 // arms fling up defensively
      P.armLp = 0.2; P.armRp = 0.2;
      break;
    }

    default: // "idle"-like neutral if unknown
      break;
  }
  return P;
}

/* compose local = translate * rot * scale, built parent-first (module helper) */
function part(parent, tx,ty,tz, rx,ry,rz, sx,sy,sz){
  let m = M4.translate(tx,ty,tz);
  if(rz) m = M4.mul(m, M4.rotZ(rz));
  if(ry) m = M4.mul(m, M4.rotY(ry));
  if(rx) m = M4.mul(m, M4.rotX(rx));
  const withScale = M4.mul(m, M4.scale(sx,sy,sz));
  return {node:parent?M4.mul(parent,m):m, draw:parent?M4.mul(parent,withScale):withScale};
}

/* ---------- SpritePipeline: the shared two-pass skeleton ----------
   Owns everything that is NOT scene-specific: the shader compile helpers, the
   fullscreen-quad buffer, the pass-1 scene target (an rt x rt RGBA texture +
   depth renderbuffer, alpha = part id) and the pass-2 "inked" post-process
   (edge-detect + posterize + pixelate) that resamples the scene into a target
   rect. RobotPipeline (below) and ShoggothPipeline (shoggoth-core.js) extend
   it with their own scene program + geometry and call:
     _beginScene()                      bind + clear the scene target, depth on
     _postPass(target, px, transparent) run pass 2 into `target` (see render())
   It does NOT own the context: it can live on a canvas of its own or inside a
   bigger renderer that draws many sprites per frame into its own framebuffers
   (the game's renderer.js).

   render() implementations leave these GL states behind, and restore nothing —
   the caller re-establishes whatever it needs afterwards:
     current program, ARRAY_BUFFER binding, the vertex-attrib arrays of the
     programs (enabled + pointed at their buffers), FRAMEBUFFER binding,
     viewport, clearColor, active texture unit (TEXTURE0) and the TEXTURE_2D
     bound on it, BLEND (disabled: pass 1 stores a part id in alpha, pass 2
     writes straight RGBA), DEPTH_TEST (disabled on exit; enabled during
     pass 1), CULL_FACE / SCISSOR_TEST (disabled). */
export class SpritePipeline {
  constructor(gl, rt, {edge=0.25} = {}){
    if(!gl) throw new Error("WebGL unavailable");
    this.gl = gl;
    this.edge = edge; // post-pass luma-gradient ink threshold

    this.postProg = this._program(postVS, postFS);
    this.pLoc = {
      aPos: gl.getAttribLocation(this.postProg,"aPos"),
      uTex: gl.getUniformLocation(this.postProg,"uTex"),
      uTexel: gl.getUniformLocation(this.postProg,"uTexel"),
      uPx: gl.getUniformLocation(this.postProg,"uPx"),
      uSize: gl.getUniformLocation(this.postProg,"uSize"),
      uTransparent: gl.getUniformLocation(this.postProg,"uTransparent"),
      uEdge: gl.getUniformLocation(this.postProg,"uEdge"),
    };
    this.quadBuf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER,this.quadBuf);
    gl.bufferData(gl.ARRAY_BUFFER,new Float32Array([-1,-1, 1,-1, -1,1, -1,1, 1,-1, 1,1]),gl.STATIC_DRAW);

    this._buildTarget(rt);
  }

  _compile(type,src){
    const gl=this.gl;
    const s=gl.createShader(type);gl.shaderSource(s,src);gl.compileShader(s);
    if(!gl.getShaderParameter(s,gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s)+"\n"+src);
    return s;
  }
  _program(vs,fs){
    const gl=this.gl;
    const p=gl.createProgram();
    gl.attachShader(p,this._compile(gl.VERTEX_SHADER,vs));
    gl.attachShader(p,this._compile(gl.FRAGMENT_SHADER,fs));
    gl.linkProgram(p);
    if(!gl.getProgramParameter(p,gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p));
    return p;
  }
  // Static geometry buffer helper (positions / normals).
  _staticBuffer(data){
    const gl=this.gl;
    const b=gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER,b); gl.bufferData(gl.ARRAY_BUFFER,data,gl.STATIC_DRAW);
    return b;
  }

  _buildTarget(RT){
    const gl=this.gl;
    this.RT = RT; // square pass-1 target
    this.rtTex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, this.rtTex);
    gl.texImage2D(gl.TEXTURE_2D,0,gl.RGBA,RT,RT,0,gl.RGBA,gl.UNSIGNED_BYTE,null);
    gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.CLAMP_TO_EDGE);
    this.depthRB = gl.createRenderbuffer();
    gl.bindRenderbuffer(gl.RENDERBUFFER, this.depthRB);
    gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT16, RT, RT);
    gl.bindRenderbuffer(gl.RENDERBUFFER, null);
    this.fbo = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.fbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.rtTex, 0);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, this.depthRB);
    if(gl.checkFramebufferStatus(gl.FRAMEBUFFER)!==gl.FRAMEBUFFER_COMPLETE) console.warn("FBO incomplete");
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  }

  // Pass 1 prologue: the scene target is bound, cleared to (0,0,0,0) (alpha 0
  // = background id), depth test on, blending off. The subclass then binds its
  // scene program + geometry and draws.
  _beginScene(){
    const gl=this.gl;
    M4.reset();
    gl.disable(gl.BLEND);        // pass 1 alpha is a part id, pass 2 writes straight RGBA
    gl.disable(gl.CULL_FACE);
    gl.disable(gl.SCISSOR_TEST);
    // (unbind TEXTURE0 first: the previous pass 2 left our scene texture there,
    //  and it is this pass's color attachment — never sample-and-render at once)
    gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, null);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.fbo);
    gl.viewport(0,0,this.RT,this.RT);
    gl.clearColor(0,0,0,0);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    gl.enable(gl.DEPTH_TEST);
  }

  // Pass 2: post -> target rect (or the whole canvas).
  //   target (optional): {fbo, x, y, w, h} — the post pass is drawn into that
  //   viewport rect of `fbo` (null = the canvas). Every pixel of the rect is
  //   overwritten (background comes out as the dark floor color, or as
  //   (0,0,0,0) when transparent), so the caller never needs to clear it.
  //   Omitted -> the whole canvas (default framebuffer), cleared first.
  _postPass(target, px, transparent){
    const gl=this.gl;
    gl.disable(gl.DEPTH_TEST);
    if(target){
      gl.bindFramebuffer(gl.FRAMEBUFFER, target.fbo || null);
      gl.viewport(target.x|0, target.y|0, target.w|0, target.h|0);
    } else {
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.viewport(0,0,gl.drawingBufferWidth,gl.drawingBufferHeight);
      if(transparent){ gl.clearColor(0,0,0,0); } else { gl.clearColor(0.03,0.04,0.05,1); }
      gl.clear(gl.COLOR_BUFFER_BIT);
    }
    gl.useProgram(this.postProg);
    gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, this.rtTex);
    gl.uniform1i(this.pLoc.uTex,0);
    gl.uniform2f(this.pLoc.uTexel, 1/this.RT, 1/this.RT);
    gl.uniform2f(this.pLoc.uSize, this.RT, this.RT);
    gl.uniform1f(this.pLoc.uPx, Math.max(1, px || 5));
    gl.uniform1f(this.pLoc.uTransparent, transparent ? 1.0 : 0.0);
    gl.uniform1f(this.pLoc.uEdge, this.edge);
    gl.bindBuffer(gl.ARRAY_BUFFER,this.quadBuf); gl.enableVertexAttribArray(this.pLoc.aPos); gl.vertexAttribPointer(this.pLoc.aPos,2,gl.FLOAT,false,0,0);
    gl.drawArrays(gl.TRIANGLES,0,6);
  }
}

/* ---------- the RobotPipeline: box-built robot on the shared skeleton ---------- */
class RobotPipeline extends SpritePipeline {
  constructor(gl, rt){
    super(gl, rt);
    this.sceneProg = this._program(sceneVS, sceneFS);
    this.sLoc = {
      aPos: gl.getAttribLocation(this.sceneProg,"aPos"),
      aNormal: gl.getAttribLocation(this.sceneProg,"aNormal"),
      uMVP: gl.getUniformLocation(this.sceneProg,"uMVP"),
      uNormalMat: gl.getUniformLocation(this.sceneProg,"uNormalMat"),
      uColor: gl.getUniformLocation(this.sceneProg,"uColor"),
      uAccent: gl.getUniformLocation(this.sceneProg,"uAccent"),
      uId: gl.getUniformLocation(this.sceneProg,"uId"),
    };
    this.cube = makeCube();
    this.posBuf = this._staticBuffer(this.cube.pos);
    this.nrmBuf = this._staticBuffer(this.cube.nrm);
  }

  // `accent` is the same palette accent for every part of one robot: it is
  // uploaded once per render(); only the body color changes (and is only
  // re-uploaded when the color array actually changes) to keep the GL call
  // count per robot low.
  _drawPart(VP, local, colBody, accent, id){
    const gl=this.gl, sLoc=this.sLoc;
    const mvp = M4.mul(VP, local);
    gl.uniformMatrix4fv(sLoc.uMVP, false, mvp);
    gl.uniformMatrix3fv(sLoc.uNormalMat, false, M4.normalFromModel(local));
    if(colBody !== this._lastColor){ gl.uniform3fv(sLoc.uColor, colBody); this._lastColor = colBody; }
    if(accent !== this._lastAccent){ gl.uniform3fv(sLoc.uAccent, accent); this._lastAccent = accent; }
    gl.uniform1f(sLoc.uId, id);
    gl.drawArrays(gl.TRIANGLES, 0, this.cube.count);
  }

  // draw a box-built weapon model anchored to the gun-hand.
  // handNode is the right forearm's "elbow" transform; we shift down to the grip.
  _drawWeapon(VP, handNode, parts, accent){
    const anchor = M4.mul(handNode, M4.translate(0, -0.42, 0.0));
    for(const b of parts){
      const local = M4.mul(anchor, M4.mul(M4.translate(b.t[0],b.t[1],b.t[2]),
                                          M4.scale(b.s[0],b.s[1],b.s[2])));
      const col = (b.c === "accent") ? accent : b.c;
      this._drawPart(VP, local, col, accent, b.id);
    }
  }
  _renderRobot(VP, pal, plan, facingRad, weapon){
    const body=pal.body, accent=pal.accent, trim=pal.trim;
    const recoil = plan.recoil || 0.0;
    const weaponParts = (weapon && weapon !== "fist") ? WEAPON_MODELS[weapon] : null;
    const holdingWeapon = !!(weaponParts && weaponParts.length);

    // root: face + backward-lean (flinch) + bob/recoil offsets
    let root = M4.mul(M4.translate(0, plan.bob, plan.zback), M4.rotY(facingRad));
    if(plan.lean) root = M4.mul(root, M4.rotX(plan.lean));

    // torso
    this._drawPart(VP, part(root, 0,1.15,0, 0,0,0, 0.9,1.0,0.55).draw, body, accent, 0.2);
    // head
    this._drawPart(VP, part(root, 0,1.95,0.02, 0,0,0, 0.62,0.55,0.55).draw, body, accent, 0.35);
    // visor strip (accent)
    this._drawPart(VP, part(root, 0,1.98,0.28, 0,0,0, 0.5,0.16,0.08).draw, accent, accent, 0.9);
    // hips
    this._drawPart(VP, part(root, 0,0.72,0, 0,0,0, 0.8,0.3,0.5).draw, trim, body, 0.25);

    // legs (pivot at hip, swing around X so they step fwd/back along Z)
    const self=this;
    function leg(sideX, ph){
      const hipPivot = M4.mul(root, M4.translate(sideX,0.6,0));
      const swung = M4.mul(hipPivot, M4.rotX(ph));
      const thigh = M4.mul(swung, M4.mul(M4.translate(0,-0.28,0), M4.scale(0.3,0.62,0.32)));
      self._drawPart(VP, thigh, body, accent, 0.4+sideX);
      const knee = M4.mul(swung, M4.translate(0,-0.6,0));
      const shinRot = M4.mul(knee, M4.rotX(Math.max(0,-ph)*0.6));
      const shin = M4.mul(shinRot, M4.mul(M4.translate(0,-0.28,0), M4.scale(0.26,0.6,0.28)));
      self._drawPart(VP, shin, trim, accent, 0.45+sideX);
      const foot = M4.mul(shinRot, M4.mul(M4.translate(0,-0.6,0.06), M4.scale(0.32,0.22,0.5)));
      self._drawPart(VP, foot, trim, body, 0.5+sideX);
    }
    leg(-0.32, plan.legA);
    leg( 0.32, plan.legB);

    // arms (pivot at shoulder)
    function arm(sideX, ph, forward, gunHand){
      const shoulder = M4.mul(root, M4.translate(sideX,1.5,0));
      let rot;
      if(forward){
        rot = M4.mul(shoulder, M4.rotX(-1.35 + recoil));
      } else {
        rot = M4.mul(shoulder, M4.rotX(ph - plan.armRaise));
      }
      const upper = M4.mul(rot, M4.mul(M4.translate(0,-0.26,0), M4.scale(0.24,0.55,0.26)));
      self._drawPart(VP, upper, body, accent, 0.6+sideX);
      const elbow = M4.mul(rot, M4.translate(0,-0.52,0));
      const fore = M4.mul(elbow, M4.mul(M4.translate(0,-0.24,0), M4.scale(0.2,0.5,0.22)));
      self._drawPart(VP, fore, trim, accent, 0.65+sideX);
      if(gunHand && holdingWeapon){
        // a held weapon replaces the bare-hand barrel
        self._drawWeapon(VP, elbow, weaponParts, accent);
      } else if(forward){
        const barrel = M4.mul(elbow, M4.mul(M4.translate(0,-0.5,0.0), M4.scale(0.14,0.5,0.14)));
        self._drawPart(VP, barrel, accent, accent, 0.95);
      }
    }
    // the right arm is the gun-hand: force it forward whenever a weapon is held,
    // so the weapon sticks out in front (like the shoot pose) from every pose.
    arm(-0.62, plan.armLp, false, false);
    arm( 0.62, plan.armRp, plan.shoot || holdingWeapon, true);
  }

  /* render one robot.
     opts: {pose, color|pal, px, time, facingDeg, weapon, transparent,
            orbit:{yaw,pitch,halfV}, halfV}
       weapon: one of WEAPONS ("fist" | "pistol" | "machinegun" | "shotgun")
       time:   continuous seconds — every value renders a distinct frame
     target (optional): {fbo, x, y, w, h} — see SpritePipeline._postPass. */
  render(opts, target){
    const gl=this.gl;
    const pose = (opts.pose || "idle");
    const pal  = opts.pal || PALETTES[(opts.color||"coral")] || PALETTES.coral;
    const time = opts.time || 0;
    const weapon = (opts.weapon in WEAPON_MODELS) ? opts.weapon : "fist";
    const facingRad = (opts.facingDeg || 0) * Math.PI/180;

    // pass 1: scene -> FBO
    this._beginScene();
    const VP = opts.orbit
      ? orbitVP(opts.orbit.yaw||0, opts.orbit.pitch||0, opts.orbit.halfV)
      : topDownVP(opts.halfV);
    const plan = posePlan(pose, time);
    gl.useProgram(this.sceneProg);
    gl.bindBuffer(gl.ARRAY_BUFFER,this.posBuf); gl.enableVertexAttribArray(this.sLoc.aPos); gl.vertexAttribPointer(this.sLoc.aPos,3,gl.FLOAT,false,0,0);
    gl.bindBuffer(gl.ARRAY_BUFFER,this.nrmBuf); gl.enableVertexAttribArray(this.sLoc.aNormal); gl.vertexAttribPointer(this.sLoc.aNormal,3,gl.FLOAT,false,0,0);
    this._lastColor = null; this._lastAccent = null; // program uniforms are ours again
    this._renderRobot(VP, pal, plan, facingRad, weapon);
    gl.disableVertexAttribArray(this.sLoc.aNormal);

    // pass 2: post -> target rect (or the whole canvas)
    this._postPass(target, opts.px, !!opts.transparent);
  }
}

/* createRobotPipeline(gl, {rt}) — the pipeline on a caller-owned context.
   rt: pass-1 scene resolution in px (square); the post pass resamples it into
   whatever target rect render() is given, so rt is the detail budget, not the
   output size. */
export function createRobotPipeline(gl, {rt=128} = {}){ return new RobotPipeline(gl, rt); }

/* ---------- CanvasRenderer: a pipeline bound to one canvas of its own ----------
   makePipeline(gl, rt) builds the pipeline (square target at canvas res). */
export class CanvasRenderer {
  constructor(canvas, makePipeline){
    this.canvas = canvas;
    const gl = canvas.getContext("webgl", {antialias:false, preserveDrawingBuffer:true});
    if(!gl) throw new Error("WebGL unavailable");
    this.gl = gl;
    this.pipeline = makePipeline(gl, canvas.width);
  }
  /* render one frame to this canvas — see the pipeline's render for opts. */
  render(opts){ this.pipeline.render(opts, null); }
}
const makeRobotPipeline = (gl, rt) => new RobotPipeline(gl, rt);
export function createRenderer(canvas){ return new CanvasRenderer(canvas, makeRobotPipeline); }

/* ---------- makeBaker: "one baked frame as a standalone canvas" factory ----------
   Returns bake(opts, size) -> HTMLCanvasElement: renders one frame through a
   shared internal canvas renderer (so repeated calls do not leak GL contexts)
   and copies it into a fresh canvas the caller owns. */
export function makeBaker(makePipeline){
  let renderer = null, rSize = 0;
  return function bake(opts, size){
    if(!renderer || rSize !== size){
      const c = document.createElement("canvas");
      c.width = c.height = size;
      renderer = new CanvasRenderer(c, makePipeline);
      rSize = size;
    }
    renderer.render(opts);
    const out = document.createElement("canvas");
    out.width = out.height = size;
    out.getContext("2d").drawImage(renderer.canvas, 0, 0);
    return out;
  };
}

/* ---------- bakeSprite: one baked robot frame as a standalone canvas ----------
   Renders ONE baked, top-down, inked/pixelated sprite frame. (The game itself
   does not bake: it runs createRobotPipeline() live inside its own context.) */
const _bakeRobot = makeBaker(makeRobotPipeline);
export function bakeSprite({pose="idle", color="coral", facingDeg=0, px=5, time=0, size=384, weapon="fist", transparent=false} = {}){
  return _bakeRobot({pose, color, px, time, facingDeg, weapon, transparent}, size); // top-down (no orbit)
}
