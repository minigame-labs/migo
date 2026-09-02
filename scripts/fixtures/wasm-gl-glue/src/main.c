// How much does one WebGL call cost when it comes from WASM through the
// Emscripten JS glue?
//
// The MigoGLX question ("is the JS glue a hotspot?") could not be asked in this
// repository because it contained no `.wasm` at all. This is the smallest thing
// that answers the measurable half: a real Emscripten build issuing a real
// WebGL command stream, so the per-call cost of the WASM -> JS -> op path can
// be timed on hardware.
//
// It is deliberately glue-heavy: many tiny draws with several uniform updates
// each, which is the shape a Unity/Emscripten export produces and the opposite
// of one big instanced draw. That biases the *ratio* toward "glue matters", so
// the number to take from it is the **per-call cost**, not "glue is a hotspot".
// Whether it is a hotspot in real content is that number times the content's
// own call count -- which the existing bunnymark/Phaser fixtures already give.
#include <emscripten.h>
#include <emscripten/html5.h>
#include <GLES3/gl3.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define DRAWS_PER_FRAME 2000

static GLuint prog, vbo;
static GLint u_offset, u_colour;
static int frame = 0;
static double window_start = 0.0;

static const char *VS =
    "#version 300 es\n"
    "layout(location=0) in vec2 a_pos;\n"
    "uniform vec2 u_offset;\n"
    "void main() { gl_Position = vec4(a_pos + u_offset, 0.0, 1.0); }\n";

static const char *FS =
    "#version 300 es\n"
    "precision mediump float;\n"
    "uniform vec4 u_colour;\n"
    "out vec4 o;\n"
    "void main() { o = u_colour; }\n";

static GLuint compile(GLenum type, const char *src) {
    GLuint s = glCreateShader(type);
    glShaderSource(s, 1, &src, NULL);
    glCompileShader(s);
    return s;
}

static void frame_cb(void) {
    glClearColor(0.06f, 0.06f, 0.08f, 1.0f);
    glClear(GL_COLOR_BUFFER_BIT);
    glUseProgram(prog);
    glBindBuffer(GL_ARRAY_BUFFER, vbo);
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, (void *)0);

    // Four glue calls per draw: two uniforms, the draw, and a state touch.
    // Unity's exports look like this -- per-object uniform updates rather than
    // one instanced call.
    for (int i = 0; i < DRAWS_PER_FRAME; i++) {
        float fx = ((i % 50) / 25.0f) - 1.0f;
        float fy = ((i / 50) / 20.0f) - 1.0f;
        glUniform2f(u_offset, fx, fy);
        glUniform4f(u_colour, (i & 1) ? 1.0f : 0.2f, 0.4f, (i & 2) ? 1.0f : 0.3f, 1.0f);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }

    frame++;
    double now = emscripten_get_now();
    if (window_start == 0.0) {
        window_start = now;
    } else if (frame % 60 == 0) {
        double fps = 60000.0 / (now - window_start);
        printf("fps=%d [wasm-gl-glue] frame %d, %d draws/frame, %d gl calls/frame\n",
               (int)(fps + 0.5), frame, DRAWS_PER_FRAME, DRAWS_PER_FRAME * 3 + 5);
        window_start = now;
    }
}

int main(void) {
    EmscriptenWebGLContextAttributes attrs;
    emscripten_webgl_init_context_attributes(&attrs);
    attrs.majorVersion = 2;
    attrs.minorVersion = 0;
    EMSCRIPTEN_WEBGL_CONTEXT_HANDLE ctx = emscripten_webgl_create_context("#canvas", &attrs);
    if (ctx <= 0) {
        printf("[wasm-gl-glue] no WebGL2 context\n");
        return 1;
    }
    emscripten_webgl_make_context_current(ctx);

    GLuint vs = compile(GL_VERTEX_SHADER, VS), fs = compile(GL_FRAGMENT_SHADER, FS);
    prog = glCreateProgram();
    glAttachShader(prog, vs);
    glAttachShader(prog, fs);
    glLinkProgram(prog);
    u_offset = glGetUniformLocation(prog, "u_offset");
    u_colour = glGetUniformLocation(prog, "u_colour");

    // One tiny triangle, reused by every draw: the point is call count, not fill.
    const float verts[] = {0.0f, 0.0f, 0.02f, 0.0f, 0.0f, 0.02f};
    glGenBuffers(1, &vbo);
    glBindBuffer(GL_ARRAY_BUFFER, vbo);
    glBufferData(GL_ARRAY_BUFFER, sizeof(verts), verts, GL_STATIC_DRAW);

    emscripten_set_main_loop(frame_cb, 0, 0);
    return 0;
}
