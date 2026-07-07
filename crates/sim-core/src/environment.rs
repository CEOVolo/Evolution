//! Deterministic uniform-grid spatial hash for neighbour queries.
//!
//! Rebuilt every tick from the SoA in index order (so the per-cell lists are deterministic).
//! Used for sensing the nearest neighbour and for contact-range predation. Queries do not
//! wrap across the toroidal edge — an edge organism simply sees fewer neighbours, which is a
//! negligible and deterministic approximation at this stage.

use crate::math::Scalar;
use crate::organism::Organisms;

#[derive(Clone, Debug)]
pub struct SpatialHash {
    cell: Scalar,
    cols: i32,
    rows: i32,
    buckets: Vec<Vec<u32>>,
}

impl SpatialHash {
    pub fn new(width: Scalar, height: Scalar, cell: Scalar) -> Self {
        let cols = (width / cell).ceil().max(1.0) as i32;
        let rows = (height / cell).ceil().max(1.0) as i32;
        SpatialHash {
            cell,
            cols,
            rows,
            buckets: vec![Vec::new(); (cols * rows) as usize],
        }
    }

    #[inline]
    fn cell_of(&self, px: Scalar, py: Scalar) -> (i32, i32) {
        let cx = ((px / self.cell) as i32).clamp(0, self.cols - 1);
        let cy = ((py / self.cell) as i32).clamp(0, self.rows - 1);
        (cx, cy)
    }

    /// Rebuild the buckets from the live organisms (index order => deterministic lists).
    pub fn rebuild(&mut self, orgs: &Organisms) {
        for b in self.buckets.iter_mut() {
            b.clear();
        }
        for i in 0..orgs.capacity() {
            if orgs.alive[i] {
                let (cx, cy) = self.cell_of(orgs.px[i], orgs.py[i]);
                self.buckets[(cy * self.cols + cx) as usize].push(i as u32);
            }
        }
    }

    /// Count live organisms within `radius` of `(px, py)`, excluding `self_slot`. Used for the
    /// density-dependent crowding penalty. Deterministic (a count is order-independent). `radius`
    /// must be `<= cell` so the 3×3 scan is complete.
    pub fn count_within(
        &self,
        orgs: &Organisms,
        self_slot: usize,
        px: Scalar,
        py: Scalar,
        radius: Scalar,
    ) -> u32 {
        let (cx, cy) = self.cell_of(px, py);
        let r2 = radius * radius;
        let mut n = 0u32;
        for gy in (cy - 1)..=(cy + 1) {
            if gy < 0 || gy >= self.rows {
                continue;
            }
            for gx in (cx - 1)..=(cx + 1) {
                if gx < 0 || gx >= self.cols {
                    continue;
                }
                for &j in &self.buckets[(gy * self.cols + gx) as usize] {
                    let j = j as usize;
                    if j == self_slot {
                        continue;
                    }
                    let dx = px - orgs.px[j];
                    let dy = py - orgs.py[j];
                    if dx * dx + dy * dy <= r2 {
                        n += 1;
                    }
                }
            }
        }
        n
    }

    /// Nearest live organism to `(px, py)` other than `self_slot`, within `radius`.
    /// Ties break by lower entity id (stable, order-independent).
    pub fn nearest(
        &self,
        orgs: &Organisms,
        self_slot: usize,
        px: Scalar,
        py: Scalar,
        radius: Scalar,
    ) -> Option<usize> {
        let (cx, cy) = self.cell_of(px, py);
        let r2 = radius * radius;
        let mut best: Option<usize> = None;
        let mut best_d = r2;
        let mut best_id = u32::MAX;
        for gy in (cy - 1)..=(cy + 1) {
            if gy < 0 || gy >= self.rows {
                continue;
            }
            for gx in (cx - 1)..=(cx + 1) {
                if gx < 0 || gx >= self.cols {
                    continue;
                }
                for &j in &self.buckets[(gy * self.cols + gx) as usize] {
                    let j = j as usize;
                    if j == self_slot {
                        continue;
                    }
                    let dx = px - orgs.px[j];
                    let dy = py - orgs.py[j];
                    let d = dx * dx + dy * dy;
                    if d < best_d || (d == best_d && orgs.id[j] < best_id) {
                        best_d = d;
                        best = Some(j);
                        best_id = orgs.id[j];
                    }
                }
            }
        }
        best
    }
}
