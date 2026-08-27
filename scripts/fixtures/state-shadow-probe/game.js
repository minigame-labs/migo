// Do the right-sized GL state shadows still dedup only what is actually
// redundant?
//
// The engine skips a driver call when its shadow says the state already matches.
// Three of those shadows were containers whose key space the spec fixes, and
// were re-shaped to match it:
//
//   * TEXTURE_2D bindings, from a `HashMap` keyed by the `GL_TEXTURE0 + i` enum
//     to an array indexed by `i`;
//   * `glEnable`/`glDisable`, from two `HashSet`s to two bitmasks;
//   * vertex-attribute pointers, enables and divisors, from three containers
//     keyed `(vao, index)` to one record per VAO with the attributes indexed
//     directly and grown to the highest index touched.
//
// Every one of those is behaviour-preserving by construction and the unit tests
// pin the predicates. What no unit test can pin is that a wrong predicate stays
// invisible: a shadow that over-dedups skips a call the driver needed, and the
// result is wrong pixels with no GL error. So each of the three gets a step here
// whose outcome is the frame's colour.
//
// Per frame:
//
//   1. clear RED
//   2. bind the SAME green texture to unit 0 and to unit 3, and sample unit 3.
//      A shadow that keyed on the texture rather than on the unit would dedup
//      the second bind away and leave unit 3 unbound -> sampling reads black.
//   3. enable(BLEND) with blendFunc(ZERO, ONE) -- which makes any draw a no-op
//      -- then disable(BLEND) before drawing. A shadow that dedups that disable
//      away leaves blending on and the draw writes nothing -> the clear stays.
//   4. draw with position at attribute 0 and a vec4 multiplier at attribute 2,
//      index 1 deliberately unused. A shadow that mis-indexes attributes leaves
//      attribute 2 disabled, the multiplier reads (0,0,0,0) -> black.
//
// Colours, all four distinguishable:
//
//   GREEN rgba(51,204,102,255)  every shadow deduped only what was redundant
//   BLACK rgba(0,0,0,255)       unit 3 unbound, or attribute 2 disabled
//   RED   rgba(217,26,38,255)   the draw wrote nothing; the blend disable was
//                               deduped away
//   BLUE  rgba(26,77,230,255)   frame one is still on the surface; presentation
//                               stopped and a green verdict would be vacuous
const canvas = migo.createCanvas();
const gl = canvas.getContext("webgl");

const VERT = `
attribute vec2 a_pos;
attribute vec4 a_mul;
varying vec2 v_uv;
varying vec4 v_mul;
void main() {
  v_uv = a_pos * 0.5 + 0.5;
  v_mul = a_mul;
  gl_Position = vec4(a_pos, 0.0, 1.0);
}
`;

// The output is the product of the sampled texel and the attribute, so either
// one going wrong is visible. A shader with any constant term would paint
// something plausible with both broken.
const FRAG = `
precision mediump float;
uniform sampler2D u_tex;
varying vec2 v_uv;
varying vec4 v_mul;
void main() {
  gl_FragColor = texture2D(u_tex, v_uv) * v_mul;
}
`;

function compile(type, src) {
  const s = gl.createShader(type);
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
    console.error("[state-shadow-probe] shader compile failed: " + gl.getShaderInfoLog(s));
  }
  return s;
}

const program = gl.createProgram();
gl.attachShader(program, compile(gl.VERTEX_SHADER, VERT));
gl.attachShader(program, compile(gl.FRAGMENT_SHADER, FRAG));
// Force the multiplier onto attribute 2 with index 1 unused, so the attribute
// shadow has to reach a non-contiguous index. Must precede the link.
const POS_INDEX = 0;
const MUL_INDEX = 2;
gl.bindAttribLocation(program, POS_INDEX, "a_pos");
gl.bindAttribLocation(program, MUL_INDEX, "a_mul");
gl.linkProgram(program);
if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
  console.error("[state-shadow-probe] link failed: " + gl.getProgramInfoLog(program));
}
gl.useProgram(program);

// Two triangles covering clip space, so the frame is one flat colour.
const posBuf = gl.createBuffer();
gl.bindBuffer(gl.ARRAY_BUFFER, posBuf);
gl.bufferData(
  gl.ARRAY_BUFFER,
  new Float32Array([-1, -1, 1, -1, -1, 1, -1, 1, 1, -1, 1, 1]),
  gl.STATIC_DRAW
);

// An all-ones multiplier per vertex. Its own buffer, so the two attributes have
// different `ARRAY_BUFFER` bindings captured -- which is the component of the
// pointer fingerprint that a shadow keyed on layout alone would drop.
const mulBuf = gl.createBuffer();
gl.bindBuffer(gl.ARRAY_BUFFER, mulBuf);
const ones = new Float32Array(6 * 4);
ones.fill(1.0);
gl.bufferData(gl.ARRAY_BUFFER, ones, gl.STATIC_DRAW);

// One green texel. Bound to two units on purpose -- see step 2.
function greenTexture() {
  const t = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, t);
  gl.texImage2D(
    gl.TEXTURE_2D,
    0,
    gl.RGBA,
    1,
    1,
    0,
    gl.RGBA,
    gl.UNSIGNED_BYTE,
    new Uint8Array([51, 204, 102, 255])
  );
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  return t;
}
const tex = greenTexture();

const SAMPLED_UNIT = 3;
const uTex = gl.getUniformLocation(program, "u_tex");
gl.uniform1i(uTex, SAMPLED_UNIT);

// ZERO/ONE: with blending on, a draw leaves the destination exactly as it was.
// That is what makes the `disable` in step 3 load-bearing rather than cosmetic.
gl.blendFunc(gl.ZERO, gl.ONE);

let frames = 0;

function paint() {
  if (frames === 0) {
    gl.clearColor(0.1, 0.3, 0.9, 1.0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    frames += 1;
    requestAnimationFrame(paint);
    return;
  }

  // 1. What the surface keeps if the draw writes nothing.
  gl.clearColor(0.85, 0.1, 0.15, 1.0);
  gl.clear(gl.COLOR_BUFFER_BIT);

  // 2. The same texture on two units. The bind to unit 3 is only redundant if
  //    the shadow forgets which unit it is talking about.
  gl.activeTexture(gl.TEXTURE0);
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.activeTexture(gl.TEXTURE0 + SAMPLED_UNIT);
  gl.bindTexture(gl.TEXTURE_2D, tex);

  // 3. Toggle the capability, so the disable has something to undo.
  gl.enable(gl.BLEND);
  gl.disable(gl.BLEND);

  // 4. Attribute 0 and attribute 2, from different buffers, index 1 unused.
  gl.useProgram(program);
  gl.bindBuffer(gl.ARRAY_BUFFER, posBuf);
  gl.enableVertexAttribArray(POS_INDEX);
  gl.vertexAttribPointer(POS_INDEX, 2, gl.FLOAT, false, 0, 0);
  gl.bindBuffer(gl.ARRAY_BUFFER, mulBuf);
  gl.enableVertexAttribArray(MUL_INDEX);
  gl.vertexAttribPointer(MUL_INDEX, 4, gl.FLOAT, false, 0, 0);
  gl.drawArrays(gl.TRIANGLES, 0, 6);

  frames += 1;
  if (frames === 2 || frames % 60 === 0) {
    console.error(
      `[state-shadow-probe] painted ${frames} frames, expect ` +
        `rgba(51,204,102,255); rgba(0,0,0,255) means unit ${SAMPLED_UNIT} lost ` +
        `its binding or attribute ${MUL_INDEX} was left disabled; ` +
        `rgba(217,26,38,255) means the blend disable was deduped away`
    );
  }
  requestAnimationFrame(paint);
}

requestAnimationFrame(paint);
