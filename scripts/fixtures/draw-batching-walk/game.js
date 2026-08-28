// Draw-call batching headroom: the case that is BOTH adjacent AND mergeable.
//
// Its sibling `draw-batching-sprite` draws the same six vertices 64 times: every
// draw is adjacent and none is mergeable, because folding them into one draw of
// 384 vertices would paint different pixels. That distinction is the whole
// reason `mergeable_draws` exists next to `adjacent_draws`.
//
// This one walks a shared buffer instead — draw i covers vertices [i*6, i*6+6),
// the way a sprite batcher packs quads. Each draw continues exactly where the
// last ended, so all 63 pairs per frame could become one draw of 384 vertices
// and paint the identical frame. `mergeable` should read ~98%, matching
// `adjacent`.
//
// The quads are stacked so the result is still a flat rgba(51,204,102,255): 64
// copies of the same full-screen pair, at 64 different buffer offsets. Same
// pixels, different ranges, which is precisely the pair of properties needed.

const canvas = migo.createCanvas();
canvas.width = 256;
canvas.height = 256;
const gl = canvas.getContext('webgl');

const DRAWS_PER_FRAME = 64;

const vs = `
  attribute vec2 a_pos;
  uniform vec2 u_offset;
  void main() { gl_Position = vec4(a_pos + u_offset, 0.0, 1.0); }
`;
const fs = `
  precision mediump float;
  uniform vec4 u_color;
  void main() { gl_FragColor = u_color; }
`;

function compile(type, src) {
  const s = gl.createShader(type);
  gl.shaderSource(s, src);
  gl.compileShader(s);
  return s;
}

const program = gl.createProgram();
gl.attachShader(program, compile(gl.VERTEX_SHADER, vs));
gl.attachShader(program, compile(gl.FRAGMENT_SHADER, fs));
gl.linkProgram(program);
gl.useProgram(program);

const buf = gl.createBuffer();
gl.bindBuffer(gl.ARRAY_BUFFER, buf);
gl.bufferData(
  gl.ARRAY_BUFFER,
  (() => {
    // 64 copies of the same full-screen triangle pair, so each draw has its own
    // six-vertex range while every draw paints the same area.
    const quad = [-1, -1, 1, -1, -1, 1, -1, 1, 1, -1, 1, 1];
    const all = new Float32Array(quad.length * DRAWS_PER_FRAME);
    for (let i = 0; i < DRAWS_PER_FRAME; i++) all.set(quad, i * quad.length);
    return all;
  })(),
  gl.STATIC_DRAW,
);
const aPos = gl.getAttribLocation(program, 'a_pos');
gl.enableVertexAttribArray(aPos);
gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 0, 0);

const uColor = gl.getUniformLocation(program, 'u_color');
const uOffset = gl.getUniformLocation(program, 'u_offset');

const GREEN = [0.2, 0.8, 0.4, 1.0];

function frame() {
  gl.viewport(0, 0, canvas.width, canvas.height);
  gl.clearColor(GREEN[0], GREEN[1], GREEN[2], GREEN[3]);
  gl.clear(gl.COLOR_BUFFER_BIT);

  // All state up front. The values do not change frame to frame either, so the
  // shadow dedups these away after frame one — which is correct and expected:
  // what is being measured is what reaches the driver, and past frame one that
  // is draws only.
  gl.uniform2f(uOffset, 0.0, 0.0);
  gl.uniform4f(uColor, GREEN[0], GREEN[1], GREEN[2], GREEN[3]);

  for (let i = 0; i < DRAWS_PER_FRAME; i++) {
    gl.drawArrays(gl.TRIANGLES, i * 6, 6);
  }

  requestAnimationFrame(frame);
}

requestAnimationFrame(frame);
