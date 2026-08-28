// Draw-call batching headroom: the CEILING.
//
// Set every piece of state once, then issue 64 draws with nothing between them.
// Every draw after the first has zero post-dedup state changes before it, so
// `adjacent_draws / draw_calls` should read ~98% (63 of 64). That is the most a
// batching pass could ever find in a frame of this size.
//
// Its sibling `draw-batching-material` is the floor: one real state change
// before every draw, so nothing is adjacent. Together they bracket the answer —
// a measured game lands between them, and the bracket is what makes the game's
// number mean something.
//
// The two fixtures are deliberately separate directories rather than one file
// branching on a mode: the engine exposes no way for JS to read an environment
// variable, and a runtime branch would be one more thing that could differ
// between the two measurements.
//
// Paints flat rgba(51, 204, 102, 255), the same verdict colour the other probes
// use, so the run can be checked for correctness too. A probe that silently
// stopped drawing would otherwise report a superb batching ratio of zero draws.

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
    gl.drawArrays(gl.TRIANGLES, 0, 6);
  }

  requestAnimationFrame(frame);
}

requestAnimationFrame(frame);
