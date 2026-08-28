// Draw-call batching headroom: the FLOOR.
//
// One real state change before every draw, the way a scene switching materials
// per object behaves. No draw has zero state changes before it, so
// `adjacent_draws / draw_calls` should read 0% — batching has nothing to work
// with, however clever the pass.
//
// Its sibling `draw-batching-sprite` is the ceiling: state set once, then 64
// untouched draws. Together they bracket the answer.
//
// The state change is `u_offset`, given a value that genuinely differs on every
// draw. That matters: a uniform re-set to the value the shadow already holds is
// deduped and never reaches the driver, which would leave the draws adjacent and
// measure the ceiling by accident. The offsets are tiny (1e-4 apart) so the
// output stays a flat colour and the run remains checkable for correctness.

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
  new Float32Array([-1, -1, 1, -1, -1, 1, -1, 1, 1, -1, 1, 1]),
  gl.STATIC_DRAW,
);
const aPos = gl.getAttribLocation(program, 'a_pos');
gl.enableVertexAttribArray(aPos);
gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 0, 0);

const uColor = gl.getUniformLocation(program, 'u_color');
const uOffset = gl.getUniformLocation(program, 'u_offset');

const GREEN = [0.2, 0.8, 0.4, 1.0];

let tick = 0;

function frame() {
  gl.viewport(0, 0, canvas.width, canvas.height);
  gl.clearColor(GREEN[0], GREEN[1], GREEN[2], GREEN[3]);
  gl.clear(gl.COLOR_BUFFER_BIT);

  gl.uniform4f(uColor, GREEN[0], GREEN[1], GREEN[2], GREEN[3]);

  // `tick` advances per frame so the offsets differ across frames as well as
  // within one — otherwise frame two's first draw would find the shadow already
  // holding frame one's first offset.
  tick = (tick + 1) % 1024;
  for (let i = 0; i < DRAWS_PER_FRAME; i++) {
    gl.uniform2f(uOffset, (tick * DRAWS_PER_FRAME + i) * 1e-6, 0.0);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
  }

  requestAnimationFrame(frame);
}

requestAnimationFrame(frame);
