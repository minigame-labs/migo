//! Compute shader sources for GPU Canvas2D tile-based rendering.

/// Tile coverage compute shader (pass 1).
///
/// Determines which tiles are covered by each path's bounding box.
/// Each invocation handles one tile and checks all paths against it,
/// producing a bitmask of covering paths (up to 32 paths per dispatch).
pub const TILE_COVERAGE_SHADER: &str = r#"#version 310 es
layout(local_size_x = 16, local_size_y = 16) in;

layout(std430, binding = 0) readonly buffer PathData {
    vec4 paths[];  // (x, y, w, h) bounding boxes
};

layout(std430, binding = 1) writeonly buffer TileCoverage {
    uint coverage[];  // bit flags per tile
};

uniform ivec2 u_canvas_size;
uniform int u_tile_size;
uniform int u_path_count;

void main() {
    ivec2 tile = ivec2(gl_GlobalInvocationID.xy);
    int tiles_x = (u_canvas_size.x + u_tile_size - 1) / u_tile_size;
    int tiles_y = (u_canvas_size.y + u_tile_size - 1) / u_tile_size;

    if (tile.x >= tiles_x || tile.y >= tiles_y) return;

    int tile_idx = tile.y * tiles_x + tile.x;
    float tx0 = float(tile.x * u_tile_size);
    float ty0 = float(tile.y * u_tile_size);
    float tx1 = tx0 + float(u_tile_size);
    float ty1 = ty0 + float(u_tile_size);

    uint mask = 0u;
    for (int i = 0; i < u_path_count && i < 32; i++) {
        vec4 p = paths[i];
        if (p.x < tx1 && p.x + p.z > tx0 && p.y < ty1 && p.y + p.w > ty0) {
            mask |= (1u << uint(i));
        }
    }
    coverage[tile_idx] = mask;
}
"#;

/// Tile compositing compute shader (pass 2).
///
/// Iterates over tiles; for each tile with nonzero coverage, writes the
/// fill color to every pixel in the tile via `imageStore`.
pub const TILE_COMPOSITE_SHADER: &str = r#"#version 310 es
layout(local_size_x = 16, local_size_y = 1) in;

layout(std430, binding = 0) readonly buffer PathData {
    vec4 paths[];
};

layout(std430, binding = 1) readonly buffer TileCoverage {
    uint coverage[];
};

layout(rgba8, binding = 0) writeonly uniform highp image2D u_output;

uniform ivec2 u_canvas_size;
uniform int u_tile_size;
uniform vec4 u_fill_color;

void main() {
    uint tile_idx = gl_GlobalInvocationID.x;
    int tiles_x = (u_canvas_size.x + u_tile_size - 1) / u_tile_size;

    if (coverage[tile_idx] == 0u) return;

    int ty = int(tile_idx) / tiles_x;
    int tx = int(tile_idx) - ty * tiles_x;
    int x0 = tx * u_tile_size;
    int y0 = ty * u_tile_size;

    for (int dy = 0; dy < u_tile_size; dy++) {
        for (int dx = 0; dx < u_tile_size; dx++) {
            int px = x0 + dx;
            int py = y0 + dy;
            if (px < u_canvas_size.x && py < u_canvas_size.y) {
                imageStore(u_output, ivec2(px, py), u_fill_color);
            }
        }
    }
}
"#;
