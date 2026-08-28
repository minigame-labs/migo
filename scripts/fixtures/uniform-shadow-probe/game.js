// Does a re-linked program's uniform reach the driver again?
//
// The engine shadows `glUniform*` per `(program, location)` to skip redundant
// uploads. A uniform's value is state of the *program object* (GLES 3.0
// §2.11.6), so `glUseProgram` cannot disturb it — but a successful
// `glLinkProgram` gives the program fresh uniform storage and initialises it,
// which throws away every value the driver held.
//
// Leave the shadow in place across that and the content's next upload of an
// unchanged value is deduped against a driver holding zero: the uniform silently
// keeps its initial value and the draw paints with it. No GL error, no log line.
// A static camera plus a re-link is enough, and re-linking is not exotic — Pixi
// v8 sorts attributes and re-links, which is why the engine tracks
// `attrib_bindings` at all.
//
// The probe makes that a colour. Per frame:
//
//   1. clear RED                            -> what the screen shows if the
//                                              draw never lands
//   2. linkProgram(P) again                 -> the driver resets P's u_color
//   3. useProgram(P), uniform4f(u_color, GREEN)
//                                           -> the *same* value as last frame
//   4. draw a full-surface quad
//
// Step 3 reaches the driver when the shadow was invalidated, and is skipped when
// it was not. So the quad is GREEN when the property holds and rgba(0,0,0,0) —
// BLACK on the presented surface — when it does not.
//
// Colours, all four distinguishable. The defect value is the one measured with
// the invalidation stubbed out, not a guess: the fragment shader writes
// `u_color` verbatim, so a uniform left at its initial value writes zero to
// every channel including alpha.
//
//   GREEN rgba(51,204,102,255)  the property holds
//   CLEAR rgba(0,0,0,0)         the upload was deduped against a reset driver
//   RED   rgba(217,26,38,255)   the draw did not land at all
//   BLUE  rgba(26,77,230,255)   frame one is still on the surface; presentation
//                               stopped and every "the screen is green" claim
//                               below would be vacuously true
//
// Nothing here reads back a pixel: `readPixels` would route through the very
// machinery a uniform defect corrupts, and it also switches the engine off its
// DrawingBuffer bypass path. The verdict is the presented frame.
const canvas = migo.createCanvas();
const gl = canvas.getContext("webgl");

const VERT = `
attribute vec2 a_pos;
void main() {
  gl_Position = vec4(a_pos, 0.0, 1.0);
}
`;

// The colour is *only* reachable through the uniform. A shader with any constant
// term would paint something plausible even with u_color left at zero, and the
// probe would report a pass for a defect.
const FRAG = `
precision mediump float;
uniform vec4 u_color;
void main() {
  gl_FragColor = u_color;
}
`;

function compile(type, src) {
  const s = gl.createShader(type);
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
    console.error("[uniform-shadow-probe] shader compile failed: " + gl.getShaderInfoLog(s));
  }
  return s;
}

const program = gl.createProgram();
gl.attachShader(program, compile(gl.VERTEX_SHADER, VERT));
gl.attachShader(program, compile(gl.FRAGMENT_SHADER, FRAG));
gl.linkProgram(program);
if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
  console.error("[uniform-shadow-probe] link failed: " + gl.getProgramInfoLog(program));
}

// Two triangles covering clip space, so the quad is the whole surface and the
// dominant sampled pixel is the fragment shader's output rather than a mix of
// quad and clear.
const quad = gl.createBuffer();
gl.bindBuffer(gl.ARRAY_BUFFER, quad);
gl.bufferData(
  gl.ARRAY_BUFFER,
  new Float32Array([-1, -1, 1, -1, -1, 1, -1, 1, 1, -1, 1, 1]),
  gl.STATIC_DRAW
);
const aPos = gl.getAttribLocation(program, "a_pos");
gl.enableVertexAttribArray(aPos);
gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 0, 0);

// Blending off: the defect writes rgba(0,0,0,0), and with blending on that would
// composite to whatever the clear left and read as RED — the wrong verdict for
// the wrong reason.
gl.disable(gl.BLEND);

let frames = 0;

function paint() {
  // 1. The colour the surface keeps if the draw does not land. Frame one is
  //    BLUE instead: "the screen is green" is an absence claim that an engine
  //    which presented once and then stopped satisfies forever, with this loop
  //    still running at 60 fps.
  if (frames === 0) {
    gl.clearColor(0.1, 0.3, 0.9, 1.0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    frames += 1;
    requestAnimationFrame(paint);
    return;
  }
  gl.clearColor(0.85, 0.1, 0.15, 1.0);
  gl.clear(gl.COLOR_BUFFER_BIT);

  // 2. The event that resets the driver's copy of u_color.
  gl.linkProgram(program);

  // 3. Re-fetch the location, as content must after a re-link, then upload the
  //    same value the shadow already holds. A one-uniform program gets the same
  //    location index back, which is exactly what makes the stale entry collide.
  gl.useProgram(program);
  const uColor = gl.getUniformLocation(program, "u_color");
  gl.uniform4f(uColor, 0.2, 0.8, 0.4, 1.0);

  // The attribute state is per-program-independent but the re-link may reset the
  // attribute binding, so re-assert it rather than trusting frame one's setup.
  gl.bindBuffer(gl.ARRAY_BUFFER, quad);
  gl.enableVertexAttribArray(aPos);
  gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 0, 0);

  // 4. GREEN if the upload reached the driver, BLACK if it was deduped away.
  gl.drawArrays(gl.TRIANGLES, 0, 6);

  frames += 1;
  if (frames === 2 || frames % 60 === 0) {
    console.error(
      `[uniform-shadow-probe] painted ${frames} frames, ` +
        `expect rgba(51,204,102,255); rgba(0,0,0,0) means the re-linked ` +
        `uniform was deduped against a reset driver`
    );
  }
  requestAnimationFrame(paint);
}

requestAnimationFrame(paint);
