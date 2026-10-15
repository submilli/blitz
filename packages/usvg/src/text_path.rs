//! Bounded curve measurement shared by every text path in one conversion.

use kurbo::{CubicBez, ParamCurve, ParamCurveArclen};

/// Each unit admits one subdivision node and its fixed-size quadrature.
const MAX_MEASUREMENTS: usize = 65_536;

#[derive(Clone)]
pub(crate) struct Work {
    remaining: usize,
    refused: bool,
}

impl Default for Work {
    fn default() -> Self {
        Self {
            remaining: MAX_MEASUREMENTS,
            refused: false,
        }
    }
}

impl Work {
    pub(crate) fn refused(&self) -> bool {
        self.refused
    }

    pub(crate) fn step(&mut self) -> Option<()> {
        if self.refused || self.remaining == 0 {
            return self.refuse();
        }
        self.remaining -= 1;
        Some(())
    }

    pub(crate) fn arclen(&mut self, curve: &CubicBez, accuracy: f64) -> Option<f64> {
        if !accuracy.is_finite() || accuracy <= 0.0 {
            return self.refuse();
        }
        self.admit(curve, accuracy, 0)?;
        let length = curve.arclen(accuracy);
        if !length.is_finite() {
            return self.refuse();
        }
        Some(length)
    }

    // Kurbo 0.13's cubic arclen_rec stops when its estimated 24-point error
    // is below accuracy, or at depth 20. That error is at most 0.0035 times
    // the control polygon length minus the chord. This conservative walk
    // therefore visits at least every node the subsequent quadrature visits.
    // It uses the same subdivision and tolerance halves, including rounding.
    fn admit(&mut self, curve: &CubicBez, accuracy: f64, depth: usize) -> Option<()> {
        self.step()?;
        let polygon = (curve.p1 - curve.p0).hypot()
            + (curve.p2 - curve.p1).hypot()
            + (curve.p3 - curve.p2).hypot();
        let excess = polygon - (curve.p3 - curve.p0).hypot();
        if !excess.is_finite() {
            return self.refuse();
        }
        if 0.0035 * excess < accuracy || depth >= 20 {
            return Some(());
        }
        let (left, right) = curve.subdivide();
        self.admit(&left, accuracy * 0.5, depth + 1)?;
        self.admit(&right, accuracy * 0.5, depth + 1)
    }

    /// Invert length with a fixed iteration bound; each measurement also
    /// consumes the document allowance. Failure refuses the conversion.
    pub(crate) fn parameter_at_length(
        &mut self,
        curve: &CubicBez,
        target: f64,
        total: f64,
        accuracy: f64,
    ) -> Option<f64> {
        if !target.is_finite() {
            return self.refuse();
        }
        if target <= 0.0 {
            return Some(0.0);
        }
        if target >= total {
            return Some(1.0);
        }
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..64 {
            let mid = (low + high) * 0.5;
            let length = self.arclen(&curve.subsegment(0.0..mid), accuracy * 0.25)?;
            if (length - target).abs() <= accuracy * 0.5 {
                return Some(mid);
            }
            if length < target {
                low = mid;
            } else {
                high = mid;
            }
        }
        self.refuse()
    }

    fn refuse<T>(&mut self) -> Option<T> {
        self.refused = true;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_curve_lengths_and_positions_retain_accuracy() {
        for curve in [
            CubicBez::new((0., 0.), (30., 0.), (70., 0.), (100., 0.)),
            CubicBez::new((0., 0.), (0., 100.), (100., 100.), (100., 0.)),
            CubicBez::new((0., 0.), (100., 200.), (-100., 200.), (0., 0.)),
        ] {
            let mut work = Work::default();
            let total = work.arclen(&curve, 0.01).unwrap();
            assert_eq!(total, curve.arclen(0.01));
            for fraction in [0., 0.01, 0.1, 0.5, 0.9, 1.] {
                let target = fraction * total;
                let t = work
                    .parameter_at_length(&curve, target, total, 0.01)
                    .unwrap();
                assert!((curve.subsegment(0.0..t).arclen(0.0001) - target).abs() < 0.01);
            }
        }
    }

    #[test]
    fn hostile_curves_and_tolerances_have_bounded_work() {
        std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(|| {
                let started = std::time::Instant::now();
                let curve = CubicBez::new((0., 0.), (0., 1e30), (1e30, 1e30), (1e30, 0.));
                let mut work = Work::default();
                assert!(work.arclen(&curve, 0.5).is_none());
                assert!(work.refused());
                let small = CubicBez::new((0., 0.), (0., 1.), (1., 1.), (1., 0.));
                assert!(Work::default().arclen(&small, 1e-30).is_none());
                assert!(started.elapsed().as_secs() < 2);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn exhausted_document_allowance_is_not_reset_per_curve() {
        let curve = CubicBez::new((0., 0.), (1., 0.), (2., 0.), (3., 0.));
        let mut work = Work {
            remaining: 2,
            refused: false,
        };
        assert_eq!(work.arclen(&curve, 0.5), Some(3.));
        assert_eq!(work.arclen(&curve, 0.5), Some(3.));
        assert!(work.arclen(&curve, 0.5).is_none());
        assert!(work.refused());
    }
}
