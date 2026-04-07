/// Source + destination rectangles for a single drawImage call within a batch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageDrawRect {
    pub sx: f32,
    pub sy: f32,
    pub sw: f32,
    pub sh: f32,
    pub dx: f32,
    pub dy: f32,
    pub dw: f32,
    pub dh: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DisplayOp {
    DrawImage {
        image_id: u32,
        sx: f32,
        sy: f32,
        sw: f32,
        sh: f32,
        dx: f32,
        dy: f32,
        dw: f32,
        dh: f32,
    },
    DrawImageBatch {
        image_id: u32,
        draws: Vec<ImageDrawRect>,
    },
}

impl DisplayOp {
    pub fn draw_image(
        image_id: u32,
        sx: f32,
        sy: f32,
        sw: f32,
        sh: f32,
        dx: f32,
        dy: f32,
        dw: f32,
        dh: f32,
    ) -> Self {
        Self::DrawImage {
            image_id,
            sx,
            sy,
            sw,
            sh,
            dx,
            dy,
            dw,
            dh,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct DisplayList {
    ops: Vec<DisplayOp>,
}

impl DisplayList {
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }

    pub fn push(&mut self, op: DisplayOp) {
        self.ops.push(op);
    }

    pub fn ops(&self) -> &[DisplayOp] {
        &self.ops
    }

    pub fn compact(self) -> Self {
        let mut out = Vec::with_capacity(self.ops.len());

        for op in self.ops {
            match op {
                DisplayOp::DrawImage {
                    image_id,
                    sx,
                    sy,
                    sw,
                    sh,
                    dx,
                    dy,
                    dw,
                    dh,
                } => {
                    let rect = ImageDrawRect { sx, sy, sw, sh, dx, dy, dw, dh };
                    match out.last_mut() {
                        Some(DisplayOp::DrawImageBatch {
                            image_id: batch_image_id,
                            draws,
                        }) if *batch_image_id == image_id => {
                            draws.push(rect);
                        }
                        Some(DisplayOp::DrawImage {
                            image_id: prev_image_id,
                            sx: prev_sx,
                            sy: prev_sy,
                            sw: prev_sw,
                            sh: prev_sh,
                            dx: prev_dx,
                            dy: prev_dy,
                            dw: prev_dw,
                            dh: prev_dh,
                        }) if *prev_image_id == image_id => {
                            let prev = ImageDrawRect {
                                sx: *prev_sx, sy: *prev_sy, sw: *prev_sw, sh: *prev_sh,
                                dx: *prev_dx, dy: *prev_dy, dw: *prev_dw, dh: *prev_dh,
                            };
                            *out.last_mut().expect("last op exists") = DisplayOp::DrawImageBatch {
                                image_id,
                                draws: vec![prev, rect],
                            };
                        }
                        _ => out.push(DisplayOp::DrawImage {
                            image_id,
                            sx,
                            sy,
                            sw,
                            sh,
                            dx,
                            dy,
                            dw,
                            dh,
                        }),
                    }
                }
                other => out.push(other),
            }
        }

        Self { ops: out }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_adjacent_draw_image_ops_for_same_texture() {
        let mut list = DisplayList::new();
        list.push(DisplayOp::draw_image(
            9, 0.0, 0.0, 16.0, 16.0, 0.0, 0.0, 32.0, 32.0,
        ));
        list.push(DisplayOp::draw_image(
            9, 16.0, 0.0, 16.0, 16.0, 32.0, 0.0, 32.0, 32.0,
        ));

        let compacted = list.compact();
        assert_eq!(compacted.ops().len(), 1);
        assert_eq!(
            compacted.ops(),
            &[DisplayOp::DrawImageBatch {
                image_id: 9,
                draws: vec![
                    ImageDrawRect { sx: 0.0, sy: 0.0, sw: 16.0, sh: 16.0, dx: 0.0, dy: 0.0, dw: 32.0, dh: 32.0 },
                    ImageDrawRect { sx: 16.0, sy: 0.0, sw: 16.0, sh: 16.0, dx: 32.0, dy: 0.0, dw: 32.0, dh: 32.0 },
                ],
            }]
        );
    }

    #[test]
    fn does_not_batch_across_different_texture_barrier() {
        let mut list = DisplayList::new();
        list.push(DisplayOp::draw_image(
            9, 0.0, 0.0, 16.0, 16.0, 0.0, 0.0, 32.0, 32.0,
        ));
        list.push(DisplayOp::draw_image(
            10, 0.0, 0.0, 16.0, 16.0, 32.0, 0.0, 32.0, 32.0,
        ));

        let compacted = list.compact();
        assert_eq!(
            compacted.ops(),
            &[
                DisplayOp::DrawImage {
                    image_id: 9,
                    sx: 0.0,
                    sy: 0.0,
                    sw: 16.0,
                    sh: 16.0,
                    dx: 0.0,
                    dy: 0.0,
                    dw: 32.0,
                    dh: 32.0,
                },
                DisplayOp::DrawImage {
                    image_id: 10,
                    sx: 0.0,
                    sy: 0.0,
                    sw: 16.0,
                    sh: 16.0,
                    dx: 32.0,
                    dy: 0.0,
                    dw: 32.0,
                    dh: 32.0,
                },
            ]
        );
    }
}
