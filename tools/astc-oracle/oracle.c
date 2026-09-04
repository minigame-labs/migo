/* Decode ASTC blocks with the platform's own decoder, and say how far off they
 * are from the pixels they were made from.
 *
 * THE POINT IS INDEPENDENCE. An encoder checked against a decoder written from
 * the same reading of the same specification proves the two agree, not that
 * either is right -- and ASTC is intricate enough that one author's misreading
 * would land in both. This uses whatever ASTC decoder the GL stack ships, which
 * on this repository's Linux host is Mesa's and on a device is the GPU's.
 *
 * Reads a file of ASTC blocks and a file of the RGBA8 pixels they encode,
 * uploads the blocks as a compressed texture, samples every texel back, and
 * reports the worst per-channel error and the peak signal-to-noise ratio.
 * Exits non-zero when the worst error exceeds the tolerance it was given.
 *
 * Build: cc oracle.c -o oracle -lEGL -lGLESv2
 */
#include <EGL/egl.h>
#include <GLES3/gl3.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define GL_COMPRESSED_RGBA_ASTC_4x4_KHR 0x93B0
#define GL_COMPRESSED_RGBA_ASTC_6x6_KHR 0x93B4
#define GL_COMPRESSED_RGBA_ASTC_8x8_KHR 0x93B7

static const char *VERTEX_SHADER =
    "#version 300 es\n"
    "const vec2 p[4] = vec2[4](vec2(-1.,-1.),vec2(1.,-1.),vec2(-1.,1.),vec2(1.,1.));\n"
    "out vec2 uv;\n"
    "void main(){ uv = p[gl_VertexID]*0.5+0.5; gl_Position = vec4(p[gl_VertexID],0.,1.); }\n";
static const char *FRAGMENT_SHADER =
    "#version 300 es\n"
    "precision highp float;\n"
    "uniform sampler2D t; in vec2 uv; out vec4 o;\n"
    "void main(){ o = texture(t, uv); }\n";

static GLuint compile(GLenum kind, const char *source) {
    GLuint shader = glCreateShader(kind);
    glShaderSource(shader, 1, &source, NULL);
    glCompileShader(shader);
    GLint ok = 0;
    glGetShaderiv(shader, GL_COMPILE_STATUS, &ok);
    if (!ok) {
        char log[1024];
        glGetShaderInfoLog(shader, sizeof log, NULL, log);
        fprintf(stderr, "shader: %s\n", log);
        exit(2);
    }
    return shader;
}

static unsigned char *slurp(const char *path, long want) {
    FILE *f = fopen(path, "rb");
    if (!f) { fprintf(stderr, "cannot open %s\n", path); exit(2); }
    unsigned char *buffer = malloc((size_t)want);
    long got = (long)fread(buffer, 1, (size_t)want, f);
    if (got != want) {
        fprintf(stderr, "%s: expected %ld bytes, read %ld\n", path, want, got);
        exit(2);
    }
    fclose(f);
    return buffer;
}

int main(int argc, char **argv) {
    if (argc < 9) {
        fprintf(stderr,
                "usage: oracle <blocks.bin> <source.rgba> <width> <height> <tolerance> "
                "<footprint> <predicted> <chosen>\n");
        return 2;
    }
    int width = atoi(argv[3]), height = atoi(argv[4]), tolerance = atoi(argv[5]);
    int side = atoi(argv[6]), predicted = atoi(argv[7]), chosen = atoi(argv[8]);
    GLenum internal_format;
    switch (side) {
        case 4: internal_format = GL_COMPRESSED_RGBA_ASTC_4x4_KHR; break;
        case 6: internal_format = GL_COMPRESSED_RGBA_ASTC_6x6_KHR; break;
        case 8: internal_format = GL_COMPRESSED_RGBA_ASTC_8x8_KHR; break;
        default:
            fprintf(stderr, "footprint must be 4, 6 or 8\n");
            return 2;
    }
    if (width <= 0 || height <= 0 || width % side || height % side) {
        fprintf(stderr, "dimensions must be positive multiples of the footprint\n");
        return 2;
    }
    long block_bytes = (long)(width / side) * (height / side) * 16;
    long pixel_bytes = (long)width * height * 4;
    unsigned char *blocks = slurp(argv[1], block_bytes);
    unsigned char *source = slurp(argv[2], pixel_bytes);

    EGLDisplay display = eglGetDisplay(EGL_DEFAULT_DISPLAY);
    if (display == EGL_NO_DISPLAY || !eglInitialize(display, NULL, NULL)) {
        fprintf(stderr, "no EGL display; the oracle cannot answer\n");
        return 4;
    }
    EGLint config_attributes[] = {
        EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES3_BIT,
        EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8, EGL_NONE,
    };
    EGLConfig config;
    EGLint count;
    if (!eglChooseConfig(display, config_attributes, &config, 1, &count) || count < 1) {
        fprintf(stderr, "no EGL config\n");
        return 4;
    }
    EGLint surface_attributes[] = { EGL_WIDTH, width, EGL_HEIGHT, height, EGL_NONE };
    EGLSurface surface = eglCreatePbufferSurface(display, config, surface_attributes);
    eglBindAPI(EGL_OPENGL_ES_API);
    EGLint context_attributes[] = { EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE };
    EGLContext context = eglCreateContext(display, config, EGL_NO_CONTEXT, context_attributes);
    if (context == EGL_NO_CONTEXT) { fprintf(stderr, "no EGL context\n"); return 4; }
    eglMakeCurrent(display, surface, surface, context);

    const char *extensions = (const char *)glGetString(GL_EXTENSIONS);
    if (!extensions || !strstr(extensions, "texture_compression_astc_ldr")) {
        /* Not a pass. A host that cannot decode ASTC cannot answer the
         * question, and reporting "skipped" as success is how a check stops
         * being one. */
        fprintf(stderr, "this GL stack (%s) does not decode ASTC; the encoder is unverified\n",
                glGetString(GL_RENDERER));
        return 4;
    }

    GLuint texture;
    glGenTextures(1, &texture);
    glBindTexture(GL_TEXTURE_2D, texture);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    glCompressedTexImage2D(GL_TEXTURE_2D, 0, internal_format, width, height, 0,
                           (GLsizei)block_bytes, blocks);
    GLenum error = glGetError();
    if (error != GL_NO_ERROR) {
        fprintf(stderr, "glCompressedTexImage2D rejected the blocks: 0x%x\n", error);
        return 5;
    }

    GLuint program = glCreateProgram();
    glAttachShader(program, compile(GL_VERTEX_SHADER, VERTEX_SHADER));
    glAttachShader(program, compile(GL_FRAGMENT_SHADER, FRAGMENT_SHADER));
    glLinkProgram(program);
    glUseProgram(program);
    GLuint vao;
    glGenVertexArrays(1, &vao);
    glBindVertexArray(vao);
    glViewport(0, 0, width, height);
    glClearColor(0, 0, 0, 1);
    glClear(GL_COLOR_BUFFER_BIT);
    glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);

    unsigned char *decoded = malloc((size_t)pixel_bytes);
    glReadPixels(0, 0, width, height, GL_RGBA, GL_UNSIGNED_BYTE, decoded);
    error = glGetError();
    if (error != GL_NO_ERROR) { fprintf(stderr, "glReadPixels: 0x%x\n", error); return 5; }

    /* Texture row 0 samples at v near zero, which is framebuffer row 0, so the
     * row index needs no flip. Getting this backwards once made a correct
     * vertical ramp read as inverted. */
    int worst = 0, worst_x = 0, worst_y = 0, worst_channel = 0;
    double squared = 0.0;
    for (int y = 0; y < height; y++) {
        for (int x = 0; x < width; x++) {
            for (int channel = 0; channel < 4; channel++) {
                size_t at = ((size_t)y * width + x) * 4 + channel;
                int difference = (int)decoded[at] - (int)source[at];
                if (difference < 0) difference = -difference;
                squared += (double)difference * difference;
                if (difference > worst) {
                    worst = difference;
                    worst_x = x;
                    worst_y = y;
                    worst_channel = channel;
                }
            }
        }
    }
    double mean_squared = squared / (double)(pixel_bytes);
    double psnr = mean_squared > 0.0 ? 10.0 * log10(255.0 * 255.0 / mean_squared) : 99.0;
    printf("  decoder: %s\n", glGetString(GL_RENDERER));
    printf("  %dx%d block %dx%d  %.2f bpp  worst %d at (%d,%d) ch%d  PSNR %.2f dB%s\n",
           width, height, side, side, 16.0 / (double)(side * side), worst, worst_x, worst_y,
           worst_channel, psnr, chosen ? "   <- chosen" : "");
    int status = 0;
    /* The budget applies to the footprint the encoder would actually pick. The
     * others are measured, printed and prediction-checked, but not held to it:
     * a 64-texel block cannot hold a hard alpha edge, and that is the fact the
     * chooser is built on rather than a fault to fail on. */
    if (chosen && worst > tolerance) {
        fprintf(stderr,
                "  the encoder chose this footprint and its worst error %d exceeds the "
                "budget of %d\n",
                worst, tolerance);
        status = 1;
    }
    /* The encoder grades its own output and picks a footprint from that grade.
     * If its model of the decoder is wrong, the grade is wrong, and the
     * selection silently ships the wrong footprint -- with the pixels to prove
     * it only on a device. Exact agreement is the bar because the model is not
     * an estimate: it is the specification's own arithmetic. */
    if (predicted >= 0 && predicted != worst) {
        fprintf(stderr,
                "  the encoder predicted a worst error of %d and the decoder produced %d; "
                "its model of the decoder is wrong, so its footprint choice is too\n",
                predicted, worst);
        status = 1;
    }
    return status;
}
