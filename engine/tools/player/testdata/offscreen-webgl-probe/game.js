// Where does an OFFSCREEN WebGL canvas actually render?
//
// `register_offscreen` gives every offscreen canvas an EGL pbuffer sized to the
// canvas, and `drawing_buffer` is `None` for it -- so `get_drawing_buffer_fbo`
// returns `None`, which means "real FBO 0", which is that pbuffer. If that is
// truly the render target, the pbuffer cannot be shrunk. This probe answers the
// question from JS: it reads back a pixel far from the origin, which only
// resolves correctly if the default framebuffer really is 64x64.
const L = (...a) => console.error("[offscreen-webgl]", ...a);

const main = migo.createCanvas();
const mctx = main.getContext("2d");

const off = migo.createCanvas();
off.width = 64;
off.height = 64;
const gl = off.getContext("webgl");

let reported = false;
function probe() {
    gl.viewport(0, 0, 64, 64);
    gl.clearColor(0.2, 0.6, 1.0, 1.0);
    gl.clear(gl.COLOR_BUFFER_BIT);

    const near = new Uint8Array(4);
    gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, near);
    const far = new Uint8Array(4);
    gl.readPixels(60, 60, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, far);

    L("near(0,0) =", near[0], near[1], near[2], near[3]);
    L("far(60,60) =", far[0], far[1], far[2], far[3]);
    L("drawingBufferWidth =", gl.drawingBufferWidth, "height =", gl.drawingBufferHeight);
    L("VERDICT", far[0] === near[0] && far[3] === 255 ? "FULL_SIZE_DEFAULT_FB" : "SMALL_OR_ABSENT_FB");
}

function loop() {
    mctx.fillStyle = "#202030";
    mctx.fillRect(0, 0, main.width, main.height);
    if (!reported) {
        reported = true;
        probe();
    }
    requestAnimationFrame(loop);
}
requestAnimationFrame(loop);
