// A deterministic sprite batch, for proving the `drawAtlas` merge is invisible.
//
// Everything here is fixed: the source image is painted by this script, the
// positions come from an integer lattice, and no clock or random number reaches
// the drawing. Two runs of this bundle must therefore produce byte-identical
// frames -- which is what makes it usable as an A/B fixture for a renderer
// change that is supposed to change nothing but the number of GL draws.
//
// The batch deliberately mixes three shapes:
//   * long runs of the same image at 1:1            -> merged
//   * a uniformly scaled run of the same image      -> merged
//   * a non-uniformly scaled sprite in the middle   -> must split the run
// so a merge that ignores its own eligibility rules shows up as moved pixels.
const L = (...a) => console.error("[sprite-batch]", ...a);

const canvas = migo.createCanvas();
const ctx = canvas.getContext("2d");
const W = canvas.width, H = canvas.height;

// Two source images, so the run also has to break on an image change. They are
// real decoded images, not canvases: `drawImage` requires a loaded `Image`, and
// a canvas source is silently dropped -- which is exactly how an earlier
// version of this fixture managed to render nothing at all while still
// reporting "768 sprites".
function load(src) {
    const img = migo.createImage();
    img.src = src;
    return img;
}
const spriteA = load("sprite-a.png");
const spriteB = load("sprite-b.png");

const COLS = 24, ROWS = 32;

function draw() {
    ctx.fillStyle = "#101014";
    ctx.fillRect(0, 0, W, H);

    let n = 0;
    for (let row = 0; row < ROWS; row++) {
        for (let col = 0; col < COLS; col++) {
            const x = 8 + col * 28;
            const y = 8 + row * 34;
            // Every 37th sprite is non-uniformly scaled: `RSXform` cannot carry
            // two different scales, so this one must fall back and split the run
            // around itself without moving its neighbours.
            if (n % 37 === 36) {
                ctx.drawImage(spriteA, 0, 0, 16, 16, x, y, 24, 12);
            } else if (row % 8 === 7) {
                // A whole row from the other image: forces an image change.
                ctx.drawImage(spriteB, 0, 0, 16, 16, x, y, 16, 16);
            } else if (col % 5 === 4) {
                // Uniform scale-up, still mergeable.
                ctx.drawImage(spriteA, 0, 0, 16, 16, x, y, 24, 24);
            } else {
                ctx.drawImage(spriteA, 0, 0, 16, 16, x, y, 16, 16);
            }
            n++;
        }
    }
}

let frames = 0;
function loop() {
    if (!spriteA.loaded || !spriteB.loaded) {
        requestAnimationFrame(loop);
        return;
    }
    draw();
    frames++;
    if (frames === 1) L("first frame drawn,", COLS * ROWS, "sprites");
    if (frames === 60) L("DONE", frames, "frames");
    requestAnimationFrame(loop);
}
requestAnimationFrame(loop);
