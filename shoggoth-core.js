"use strict";
/* =========================================================================
   OPEN MIAMI - the FLOOR 13½ boss: the SHOGGOTH, 3D -> stylized 2D.
   Vanilla WebGL 1, no libraries. Built on robot-core.js's SpritePipeline
   (same pass-1 target, same "inked" edge/posterize/pixelate post pass, same
   pooled mat4 helpers) — only the scene is its own: a writhing mass of
   overlapping spheres, a friendly yellow smiley mask riding on the crown,
   and, once the mask is consumed, lashing tentacles studded with pale-yellow
   dot eyes.

   Exports:
     PHASES                            - ["masked", "transition", "enraged"]
     MASK_OFF_SECS                     - the mask-off animation length (3.4 s;
                                         mirrors src/systems/boss.rs)
     SPHERE_TESS / DEFAULT_TESS        - sphere tessellation presets
                                         ("high" 16x20, "low" 8x10; the game
                                         default is LOW)
     createShoggothPipeline(gl, {rt, tess}) - the pipeline on an EXISTING GL
         context:
         .render(opts, target) draws one shoggoth into a caller-provided
         framebuffer rect {fbo,x,y,w,h} (or the whole canvas when omitted),
         transparent background when opts.transparent. This is what the game
         renderer runs live, every frame, inside its own GL context.
           opts: {
             phase:   "masked" | "transition" | "enraged"   (default "masked")
             reveal:  0..1 mask-off progress; overrides phase when given
                      (0 = mask intact, 1 = raw form). "transition" without a
                      reveal plays it from time (t / MASK_OFF_SECS).
             time:    continuous seconds — drives all the writhing
             heading: radians, the direction it is moving in the XZ / screen
                      plane (0 = +x/right, PI/2 = +z/screen-down); the mask
                      leans toward it. Default: the wander behaviour's heading.
             lookUp:  0..1, how flat toward the camera the mask tilts (the
                      "it notices you" stare). Default: the wander behaviour's
                      periodic look-up beat, forced to 1 by the transition.
             wander:  true -> the mass also DRIFTS on the floor along the
                      behaviour's heading (the inspector's masked idle). The
                      game passes false: position comes from the simulation.
             px:      pixelation block size (post pass), default 5
             tess:    "low" | "high" — switch the sphere preset live
                      (rebuilds the shared buffers only when it changes)
             transparent, orbit:{yaw,pitch,halfV}, halfV — as robot-core
           }
     createShoggothRenderer(canvas)    - a CanvasRenderer of the pipeline
     bakeShoggoth({...opts, size})     - one baked top-down frame -> canvas
   ========================================================================= */

import { M4, SpritePipeline, CanvasRenderer, makeBaker, orbitVP } from "./robot-core.js";

export const PHASES = ["masked", "transition", "enraged"];
/* seconds of mask-off animation (masked -> raw); src/systems/boss.rs drives
   the in-game `reveal` over the same duration */
export const MASK_OFF_SECS = 3.4;

/* ---------- palette (the boss is not recolored) ---------- */
const C_BODY   = [0.11,0.14,0.13];  // dark flesh
const C_BODYA  = [0.17,0.22,0.19];  // its top-lit accent
const C_PURPLE = [0.14,0.10,0.17];  // bruised purple lobes
const C_PURPLEA= [0.20,0.15,0.24];
const C_TIP    = [0.34,0.12,0.13];  // tentacle tips, wet red
const C_TIPA   = [0.48,0.18,0.17];
const C_MASK   = [1.00,0.83,0.14];  // friendly yellow smiley
const C_INK    = [0.05,0.04,0.02];  // dark features on the mask
const C_YEYE   = [1.00,0.95,0.55];  // pale-yellow dot eyes (raw form)
const C_YHOT   = [1.00,1.00,0.86];  // hot core of the dot

function clamp01(x){return x<0?0:(x>1?1:x);}
function smoothstep(a,b,x){const t=clamp01((x-a)/(b-a));return t*t*(3-2*t);}
function ease(x){return smoothstep(0,1,x);}
function mix(a,b,t){return a+(b-a)*t;}
/* deterministic pseudo-random */
function hash(i){ const s=Math.sin(i*127.1+0.7)*43758.5453; return s-Math.floor(s); }

/* ---------- scene shader: lit surfaces + per-part id in alpha (for edges),
   plus an emissive term so the yellow mask and eyes glow flat through the
   posterize. Same lighting model as robot-core's scene shader. ---------- */
const sceneVS = `
attribute vec3 aPos;
attribute vec3 aNormal;
uniform mat4 uMVP;
uniform mat3 uNormalMat;
varying vec3 vN;
void main(){
  vec4 p = uMVP * vec4(aPos,1.0);
  gl_Position = p;
  vN = normalize(uNormalMat * aNormal);
}
`;
const sceneFS = `
precision mediump float;
varying vec3 vN;
uniform vec3 uColor;
uniform vec3 uAccent;
uniform float uId;
uniform float uEmis;   // 0 = fully lit, 1 = flat glow color
void main(){
  vec3 L = normalize(vec3(0.35, 0.9, 0.45));
  float ndl = max(dot(normalize(vN), L), 0.0);
  float amb = 0.35;
  float shade = amb + ndl*0.75;
  vec3 base = mix(uColor, uAccent, clamp(vN.y*0.5+0.2,0.0,1.0)*0.5);
  vec3 col = base * shade;
  col = mix(col, uColor, uEmis);
  gl_FragColor = vec4(col, uId);
}
`;

/* ---------- unit sphere geometry (positions + normals) ---------- */
function makeSphere(stacks, slices){
  const p=[], n=[];
  function vert(i,j){
    const v = i/stacks, u = j/slices;
    const phi = v*Math.PI;
    const th  = u*2*Math.PI;
    return [Math.sin(phi)*Math.cos(th), Math.cos(phi), Math.sin(phi)*Math.sin(th)];
  }
  for(let i=0;i<stacks;i++){
    for(let j=0;j<slices;j++){
      const a=vert(i,j), b=vert(i,j+1), c=vert(i+1,j+1), d=vert(i+1,j);
      for(const v of [a,b,c, a,c,d]){ p.push(v[0],v[1],v[2]); n.push(v[0],v[1],v[2]); }
    }
  }
  return {pos:new Float32Array(p), nrm:new Float32Array(n), count:p.length/3};
}

/* Sphere tessellation presets. The boss is ~130 sphere instances per frame,
   all inked + pixelated into a small tile, so LOW (8x10) is visually
   identical to HIGH (the legacy 16x20) at game size for ~1/4 the vertices.
   The GAME default is LOW; the inspector has a TESS toggle to compare
   (render opts.tess or createShoggothPipeline's {tess}). */
export const SPHERE_TESS = { high:[16,20], low:[8,10] };
export const DEFAULT_TESS = "low";

/* ---------- camera: slightly-tilted top-down (the boss reads best with a hint
   of the mask's face). The half-extent frames the whole raw form: the mass
   (radius ~2.5 with its lobes) plus most of the tentacles' reach, so the
   in-game tile does not cut them off square. ---------- */
const CAM_HALF_V = 3.8;
const CAM_CENTER = [0,0.4,0];
function bossVP(halfV){
  halfV = halfV || CAM_HALF_V;
  const proj = M4.ortho(-halfV,halfV,-halfV,halfV,0.1,60);
  const eye=[0, 12, 6.1], up=[0,0,-1];
  return M4.mul(proj, M4.lookAt(eye,CAM_CENTER,up));
}

/* =========================================================================
   WANDER BEHAVIOUR (masked phase)
   A tiny deterministic state machine, stepped at a fixed dt. It drifts along
   a heading, occasionally stops and looks up, then picks a new heading. The
   sim state is cached per pipeline so live playback is cheap and a frame at
   any `time` is reproducible (re-simulated from 0 when time goes backward).
   ========================================================================= */
function newBeh(){
  return { t:0, x:0, z:0, heading:hash(1)*6.283, tgt:hash(1)*6.283,
           mode:0, modeT:0, dur:3.5+hash(2)*2.5, look:0, dec:1 };
}
function stepBeh(b,dt){
  b.t+=dt; b.modeT+=dt;
  if(b.mode===0){                       // drifting
    let d=b.tgt-b.heading; while(d>Math.PI)d-=6.283; while(d<-Math.PI)d+=6.283;
    b.heading += d*Math.min(1,dt*1.6);  // steer smoothly toward target heading
    const spd=0.55;
    b.x += Math.cos(b.heading)*spd*dt;
    b.z += Math.sin(b.heading)*spd*dt;
    if(Math.hypot(b.x,b.z)>1.35){        // stay in frame: steer back inward
      b.tgt = Math.atan2(-b.z,-b.x) + (hash(b.dec+7)-0.5)*0.8;
    }
    b.look += (0-b.look)*Math.min(1,dt*3.0);
    if(b.modeT>b.dur){ b.mode=1; b.modeT=0; b.dur=1.8+hash(b.dec+3)*1.6; b.dec++; }
  } else {                              // stopped, looking up
    b.look += (1-b.look)*Math.min(1,dt*2.6);
    if(b.modeT>b.dur){
      b.mode=0; b.modeT=0; b.dur=3.2+hash(b.dec+3)*2.6;
      b.tgt = b.heading + (hash(b.dec+5)-0.5)*3.2; b.dec++;
    }
  }
}

/* =========================================================================
   The pipeline
   ========================================================================= */
class ShoggothPipeline extends SpritePipeline {
  constructor(gl, rt, tess){
    super(gl, rt, {edge:0.30});
    this.sceneProg = this._program(sceneVS, sceneFS);
    this.sLoc = {
      aPos: gl.getAttribLocation(this.sceneProg,"aPos"),
      aNormal: gl.getAttribLocation(this.sceneProg,"aNormal"),
      uMVP: gl.getUniformLocation(this.sceneProg,"uMVP"),
      uNormalMat: gl.getUniformLocation(this.sceneProg,"uNormalMat"),
      uColor: gl.getUniformLocation(this.sceneProg,"uColor"),
      uAccent: gl.getUniformLocation(this.sceneProg,"uAccent"),
      uId: gl.getUniformLocation(this.sceneProg,"uId"),
      uEmis: gl.getUniformLocation(this.sceneProg,"uEmis"),
    };
    this.tess = null;
    this.setTess(tess || DEFAULT_TESS);
    this.simState = null; this.simT = -1;
    this.VP = null;
  }

  /* switch the sphere tessellation preset ("low" | "high"); rebuilds the
     shared unit-sphere buffers, no-op when already on that preset. */
  setTess(name){
    if(!SPHERE_TESS[name]) name = DEFAULT_TESS;
    if(name === this.tess) return;
    const gl=this.gl;
    if(this.posBuf) gl.deleteBuffer(this.posBuf);
    if(this.nrmBuf) gl.deleteBuffer(this.nrmBuf);
    this.tess = name;
    const [stacks, slices] = SPHERE_TESS[name];
    this.sphere = makeSphere(stacks, slices);
    this.posBuf = this._staticBuffer(this.sphere.pos);
    this.nrmBuf = this._staticBuffer(this.sphere.nrm);
  }

  /* wander behaviour state at time t (cached, deterministic) */
  _behaviorAt(t){
    if(this.simState===null || t < this.simT-1e-6){ this.simState=newBeh(); this.simT=0; }
    const dt=1/60; let guard=0;
    while(this.simT < t-1e-9 && guard<300000){
      const s=Math.min(dt, t-this.simT); stepBeh(this.simState,s); this.simT+=s; guard++;
    }
    return this.simState;
  }

  /* ---------- draw one sphere instance ---------- */
  _sphere(model, colBody, accent, id, emis){
    const gl=this.gl, sLoc=this.sLoc;
    gl.uniformMatrix4fv(sLoc.uMVP, false, M4.mul(this.VP, model));
    gl.uniformMatrix3fv(sLoc.uNormalMat, false, M4.normalFromModel(model));
    gl.uniform3fv(sLoc.uColor, colBody);
    gl.uniform3fv(sLoc.uAccent, accent);
    gl.uniform1f(sLoc.uId, id);
    gl.uniform1f(sLoc.uEmis, emis||0.0);
    gl.drawArrays(gl.TRIANGLES, 0, this.sphere.count);
  }
  _blob(root, x,y,z, rx,ry,rz, col,acc, id, emis){
    const m = M4.mul(root, M4.mul(M4.translate(x,y,z), M4.scale(rx,ry,rz)));
    this._sphere(m, col, acc, id, emis);
  }

  /* ---------- THE WRITHING MASS: 1 core + orbiting satellite lobes ---------- */
  _drawMass(root, time, frantic){
    const LOBES = 9;
    const coreSq = 1.0 + 0.06*Math.sin(time*0.9);
    this._blob(root, 0,0,0, 1.65, 1.35*coreSq, 1.65, C_BODY, C_BODYA, 0.14, 0.0);
    this._blob(root, 0.25*Math.sin(time*0.6), 0.15, -0.2*Math.cos(time*0.5),
         1.25,1.15,1.3, C_PURPLE, C_PURPLEA, 0.22, 0.0);
    for(let k=0;k<LOBES;k++){
      const a = (k/LOBES)*Math.PI*2;
      const spd = 0.4 + hash(k)*0.5;
      const ph = hash(k+10)*6.28;
      const wob = frantic ? 0.55 : 0.28;
      const rad = 1.15 + hash(k+3)*0.5 + Math.sin(time*spd+ph)*wob;
      const yb  = -0.35 + Math.sin(time*spd*1.3+ph)*(frantic?0.5:0.28) + hash(k+7)*0.5;
      const x = Math.cos(a + time*(frantic?0.5:0.22))*rad;
      const z = Math.sin(a + time*(frantic?0.5:0.22))*rad;
      const r = 0.62 + hash(k+5)*0.45;
      const pr = 1.0 + 0.14*Math.sin(time*1.7+ph);
      const purple = hash(k+2) > 0.55;
      this._blob(root, x, yb, z, r*pr, r*(0.85+0.2*Math.sin(time+ph)), r*pr,
           purple?C_PURPLE:C_BODY, purple?C_PURPLEA:C_BODYA, 0.30 + k*0.055, 0.0);
    }
  }

  /* ---------- THE SMILEY MASK / MASK-OFF TRANSITION ----------
     An assembly of yellow shards (a ring of wedges + a centre cap) plus the
     ink features. At reveal=0 the shards overlap into a clean dome (the intact
     mask). As reveal climbs the shell cracks, then the shards and the features
     are SUCKED INWARD and down into the maw — shrinking, spiralling, darkening
     to dead flesh — uncovering the raw form. `mroot` already places/rotates
     the mask on the crown (heading + look tilt). */
  _drawMaskAssembly(mroot, time, reveal){
    const s = ease(clamp01(reveal*1.05));   // consume progress 0..1
    const jitter = (reveal>0.02 && reveal<0.55) ? (reveal*0.05) : 0.0;
    const dk = smoothstep(0.42,1.0,reveal);
    const shardCol = [mix(C_MASK[0],C_BODY[0],dk*0.95), mix(C_MASK[1],C_BODY[1],dk*0.95), mix(C_MASK[2],C_BODY[2],dk*0.95)];
    const shardEm  = 0.85*(1.0 - smoothstep(0.30,1.0,reveal));
    const shrink   = mix(1.0, 0.06, s);     // shards shrink as they are pulled in
    const inR      = 1.0 - s;               // ring radius collapses toward the maw
    const sink     = s*0.6;                 // slight downward drift into the maw

    // hairline cracks that appear just before the shell lets go
    if(reveal>0.02 && reveal<0.5){
      const ca = smoothstep(0.02,0.16,reveal) * (1.0-smoothstep(0.36,0.5,reveal));
      for(let c=0;c<3;c++){
        const ang=c*1.05+0.3;
        let m=M4.mul(mroot, M4.translate(0,0.30,0));
        m=M4.mul(m, M4.rotY(ang));
        m=M4.mul(m, M4.scale(1.15*ca, 0.05, 0.055));
        this._sphere(m, C_INK, C_INK, 0.58, 0.0);
      }
    }

    // ring of wedge shards: sucked inward + down while spiralling
    const SH=6;
    for(let k=0;k<SH;k++){
      const a=(k/SH)*Math.PI*2;
      const swirl=a + s*2.4*((k%2)?1:-1);          // spiral into the maw
      const jx=Math.sin(time*23+k)*jitter, jz=Math.cos(time*21+k)*jitter;
      const px=Math.cos(swirl)*0.60*inR + jx;
      const pz=Math.sin(swirl)*0.60*inR + jz;
      const py=-sink*(0.6+hash(k+41)*0.5);
      const spin=s*(4.0+hash(k+42)*3.0);
      let m=M4.translate(px,py,pz);
      m=M4.mul(m, M4.rotZ(spin*((k%2)?1:-1)));
      m=M4.mul(m, M4.rotX(spin*0.6));
      m=M4.mul(m, M4.scale(0.62*shrink,0.26*shrink,0.62*shrink));
      this._sphere(M4.mul(mroot,m), shardCol, shardCol, 0.80+k*0.006, shardEm);
    }
    // centre cap: shrinks down into the maw
    {
      let m=M4.translate(0, -sink*0.8, 0);
      m=M4.mul(m, M4.rotX(s*4.0));
      m=M4.mul(m, M4.scale(0.66*shrink,0.30*shrink,0.66*shrink));
      this._sphere(M4.mul(mroot,m), shardCol, shardCol, 0.79, shardEm);
    }

    // ink features (two eyes + upward smile) are drawn in toward the centre
    const fy=0.42;
    const feat=(x,z)=>{
      let m=M4.translate(x*inR, fy - sink, z*inR);
      m=M4.mul(m, M4.rotZ(s*5.0));
      m=M4.mul(m, M4.scale(shrink,shrink,shrink));
      return M4.mul(mroot,m);
    };
    this._sphere(M4.mul(feat(-0.44,-0.42), M4.scale(0.20,0.15,0.24)), C_INK,C_INK,0.55,0.0);
    this._sphere(M4.mul(feat( 0.44,-0.42), M4.scale(0.20,0.15,0.24)), C_INK,C_INK,0.55,0.0);
    const N=7;
    for(let i=0;i<N;i++){
      const tt=(i/(N-1))*2-1;
      const x=tt*0.66;
      const z=0.16+0.34*(1.0-tt*tt);
      this._sphere(M4.mul(feat(x,z), M4.scale(0.135,0.12,0.135)), C_INK,C_INK,0.55,0.0);
    }
  }

  /* ---------- RAW FORM: pale-yellow glowing dot eyes over the crown ---------- */
  _drawYellowEyes(root, time, fade){
    const N=15;
    for(let i=0;i<N;i++){
      const a=i*2.399;                       // golden-angle scatter
      const rr=0.18 + hash(i+30)*0.92;       // cluster over the crown
      const x=Math.cos(a)*rr + 0.08*Math.sin(time*1.2+i);
      const z=Math.sin(a)*rr*0.9 + 0.08*Math.cos(time*1.0+i);
      const y=1.35 + hash(i+31)*0.65;        // up on the crown, in front of the cam
      const tw=0.85 + 0.15*Math.abs(Math.sin(time*2.5 + i*1.3)); // gentle twinkle
      // big enough that the yellow core survives the ink outline at this px size
      const s=(0.16 + hash(i+32)*0.07) * (0.55+0.45*fade);
      this._blob(root, x,y,z, s,s,s, C_YEYE, C_YEYE, 0.62+i*0.012, tw*fade);
      this._blob(root, x,y+0.03,z, s*0.45,s*0.45,s*0.45, C_YHOT, C_YHOT, 0.92, fade);
    }
  }

  /* many chunky lashing tentacles. `grow` scales them in during the transition
     (0 -> hidden, 1 -> full length). `eyeFade` fades in the little pale-yellow
     dot-eyes that also stud the arms & tips. */
  _drawTentacles(root, time, grow, frantic, eyeFade){
    const T=11, SEG=8;
    for(let k=0;k<T;k++){
      const a=(k/T)*Math.PI*2 + 0.3;
      let m=M4.mul(root, M4.translate(Math.cos(a)*1.20, -0.05, Math.sin(a)*1.20));
      m=M4.mul(m, M4.rotY(-a));               // face outward
      m=M4.mul(m, M4.rotZ(-1.05));            // tip the up-axis outward
      const seglen=0.60*grow;
      for(let i=0;i<SEG;i++){
        const bend = Math.sin(time*2.5 + i*0.9 + k*1.7)*(frantic?0.62:0.4);
        const sweep= Math.cos(time*1.9 + i*0.7 + k*2.1)*(frantic?0.55:0.35);
        m=M4.mul(m, M4.rotX(sweep));
        m=M4.mul(m, M4.rotZ(bend*0.5));
        const tp=i/(SEG-1);
        const r=0.50*(1.0 - tp*0.70);
        const seg=M4.mul(m, M4.mul(M4.translate(0, seglen*0.5, 0), M4.scale(r, seglen*0.62, r)));
        const col=[mix(C_BODY[0],C_TIP[0],tp), mix(C_BODY[1],C_TIP[1],tp), mix(C_BODY[2],C_TIP[2],tp)];
        const acc=[mix(C_BODYA[0],C_TIPA[0],tp),mix(C_BODYA[1],C_TIPA[1],tp),mix(C_BODYA[2],C_TIPA[2],tp)];
        this._sphere(seg, col, acc, 0.30 + k*0.03 + i*0.005, 0.0);
        // dot-eyes on the arm: always one at the tip, plus a few scattered along
        if(eyeFade>0.02 && grow>0.6 && (i===SEG-1 || hash(k*13+i+50) > 0.62)){
          const es = Math.max(0.13, r*0.75);
          const tw = 0.82 + 0.18*Math.abs(Math.sin(time*2.6 + k*1.3 + i));
          this._blob(m, 0, seglen*0.5, r*0.85, es,es,es, C_YEYE, C_YEYE, 0.66 + k*0.02 + i*0.006, tw*eyeFade);
          this._blob(m, 0, seglen*0.55, r*0.9,  es*0.45,es*0.45,es*0.45, C_YHOT, C_YHOT, 0.94, eyeFade);
        }
        m=M4.mul(m, M4.translate(0, seglen, 0));
      }
    }
  }

  /* ---------- the whole boss for one frame (pass 1 scene) ---------- */
  _renderShoggoth(time, reveal, heading, lookUp, drift){
    const gl=this.gl;
    // drift eases to centre as it rears up; spin & list ramp in with the raw form
    const dr    = 1.0-ease(reveal);
    const spin  = ease(reveal)*0.32;
    const list  = 0.05 + ease(reveal)*0.07;
    let bodyRoot = M4.translate(drift[0]*dr, 0, drift[1]*dr);
    bodyRoot = M4.mul(bodyRoot, M4.rotY(time*spin));
    bodyRoot = M4.mul(bodyRoot, M4.rotZ(Math.sin(time*0.5)*list));
    bodyRoot = M4.mul(bodyRoot, M4.rotX(Math.cos(time*0.42)*list));

    const frantic = reveal>0.5;
    const grow    = smoothstep(0.12,0.95,reveal);
    const eyeFade = smoothstep(0.35,1.0,reveal);

    if(grow>0.02)     this._drawTentacles(bodyRoot, time, grow, frantic, eyeFade);
    this._drawMass(bodyRoot, time, frantic);
    if(eyeFade>0.02)  this._drawYellowEyes(bodyRoot, time, eyeFade);

    // the mask / its break-up (skip once fully gone)
    if(reveal < 0.999){
      const yTop=1.55;                       // ride high on the crown, above the lobes
      const lean=(1.0-lookUp)*0.34;          // lean toward heading when moving; flat when looking up
      let mroot=M4.mul(bodyRoot, M4.translate(0,yTop,0.05));
      // face (the smile side, local +z) toward the heading, then tilt forward
      mroot=M4.mul(mroot, M4.rotY(Math.PI/2 - heading));
      mroot=M4.mul(mroot, M4.rotX(lean));
      // Draw the mask with depth-test OFF so (a) in masked/look-up it is ALWAYS in
      // front of the mass/lobes at every heading, and (b) during the transition the
      // shards stay visible as they spiral inward and are consumed.
      gl.disable(gl.DEPTH_TEST);
      this._drawMaskAssembly(mroot, time, reveal);
      gl.enable(gl.DEPTH_TEST);
    }
  }

  /* render one shoggoth — opts / target: see the module header. */
  render(opts, target){
    const gl=this.gl;
    if(opts.tess) this.setTess(opts.tess);
    const time = opts.time || 0;
    const phase = PHASES.includes(opts.phase) ? opts.phase : "masked";
    let reveal;
    if(typeof opts.reveal === "number" && !Number.isNaN(opts.reveal)) reveal = clamp01(opts.reveal);
    else reveal = phase==="enraged" ? 1.0 : (phase==="transition" ? clamp01(time/MASK_OFF_SECS) : 0.0);

    // behaviour: heading / look-up beat / drift, unless the caller drives them
    const b = this._behaviorAt(time);
    const heading = (typeof opts.heading === "number") ? opts.heading : b.heading;
    let lookUp = (typeof opts.lookUp === "number") ? clamp01(opts.lookUp) : b.look;
    // it rears up to stare as the mask lets go
    if(reveal>0 && reveal<1) lookUp = Math.max(lookUp, 1.0 - smoothstep(0.06, 0.32, reveal));
    const drift = opts.wander ? [b.x, b.z] : [0, 0];

    // pass 1: scene -> FBO
    this._beginScene();
    this.VP = opts.orbit
      ? orbitVP(opts.orbit.yaw||0, opts.orbit.pitch||0, opts.orbit.halfV||CAM_HALF_V, CAM_CENTER)
      : bossVP(opts.halfV);
    gl.useProgram(this.sceneProg);
    gl.bindBuffer(gl.ARRAY_BUFFER,this.posBuf); gl.enableVertexAttribArray(this.sLoc.aPos); gl.vertexAttribPointer(this.sLoc.aPos,3,gl.FLOAT,false,0,0);
    gl.bindBuffer(gl.ARRAY_BUFFER,this.nrmBuf); gl.enableVertexAttribArray(this.sLoc.aNormal); gl.vertexAttribPointer(this.sLoc.aNormal,3,gl.FLOAT,false,0,0);
    this._renderShoggoth(time, reveal, heading, lookUp, drift);
    gl.disableVertexAttribArray(this.sLoc.aNormal);

    // pass 2: post -> target rect (or the whole canvas)
    this._postPass(target, opts.px, !!opts.transparent);
  }
}

/* createShoggothPipeline(gl, {rt}) — the pipeline on a caller-owned context.
   rt: pass-1 scene resolution in px (square); the post pass resamples it into
   whatever target rect render() is given, so rt is the detail budget, not the
   output size. The boss is big: 256 is a good default. */
export function createShoggothPipeline(gl, {rt=256, tess=DEFAULT_TESS} = {}){ return new ShoggothPipeline(gl, rt, tess); }

const makeShoggothPipeline = (gl, rt) => new ShoggothPipeline(gl, rt);
/* a CanvasRenderer bound to one canvas (owns a context + a pipeline) */
export function createShoggothRenderer(canvas){ return new CanvasRenderer(canvas, makeShoggothPipeline); }

/* bakeShoggoth({phase, reveal, time, heading, lookUp, wander, px, size, transparent})
   -> HTMLCanvasElement: ONE baked top-down frame (the inspector's 2D view). */
const _bakeShoggoth = makeBaker(makeShoggothPipeline);
export function bakeShoggoth({size=384, ...opts} = {}){
  const o = Object.assign({}, opts);
  delete o.orbit; // top-down only
  return _bakeShoggoth(o, size);
}
