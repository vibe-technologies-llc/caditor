pub trait Maths: Sized {
    fn sin_libm(self) -> Self;
    fn cos_libm(self) -> Self;
    fn tan_libm(self) -> Self;
    fn sin_cos_libm(self) -> (Self, Self);
    fn asin_libm(self) -> Self;
    fn acos_libm(self) -> Self;
    fn atan_libm(self) -> Self;
    fn atan2_libm(self, x: Self) -> Self;
    fn sinh_libm(self) -> Self;
    fn cosh_libm(self) -> Self;
    fn tanh_libm(self) -> Self;
    fn asinh_libm(self) -> Self;
    fn acosh_libm(self) -> Self;
    fn atanh_libm(self) -> Self;
    fn exp_libm(self) -> Self;
    fn exp2_libm(self) -> Self;
    fn exp_m1_libm(self) -> Self;
    fn ln_libm(self) -> Self;
    fn ln_1p_libm(self) -> Self;
    fn log2_libm(self) -> Self;
    fn log10_libm(self) -> Self;
    fn powf_libm(self, exponent: Self) -> Self;
    fn hypot_libm(self, other: Self) -> Self;
    fn cbrt_libm(self) -> Self;
}

impl Maths for f64 {
    fn sin_libm(self) -> f64 {
        libm::sin(self)
    }

    fn cos_libm(self) -> f64 {
        libm::cos(self)
    }

    fn tan_libm(self) -> f64 {
        libm::tan(self)
    }

    fn sin_cos_libm(self) -> (f64, f64) {
        libm::sincos(self)
    }

    fn asin_libm(self) -> f64 {
        libm::asin(self)
    }

    fn acos_libm(self) -> f64 {
        libm::acos(self)
    }

    fn atan_libm(self) -> f64 {
        libm::atan(self)
    }

    fn atan2_libm(self, x: f64) -> f64 {
        libm::atan2(self, x)
    }

    fn sinh_libm(self) -> f64 {
        libm::sinh(self)
    }

    fn cosh_libm(self) -> f64 {
        libm::cosh(self)
    }

    fn tanh_libm(self) -> f64 {
        libm::tanh(self)
    }

    fn asinh_libm(self) -> f64 {
        libm::asinh(self)
    }

    fn acosh_libm(self) -> f64 {
        libm::acosh(self)
    }

    fn atanh_libm(self) -> f64 {
        libm::atanh(self)
    }

    fn exp_libm(self) -> f64 {
        libm::exp(self)
    }

    fn exp2_libm(self) -> f64 {
        libm::exp2(self)
    }

    fn exp_m1_libm(self) -> f64 {
        libm::expm1(self)
    }

    fn ln_libm(self) -> f64 {
        libm::log(self)
    }

    fn ln_1p_libm(self) -> f64 {
        libm::log1p(self)
    }

    fn log2_libm(self) -> f64 {
        libm::log2(self)
    }

    fn log10_libm(self) -> f64 {
        libm::log10(self)
    }

    fn powf_libm(self, exponent: f64) -> f64 {
        libm::pow(self, exponent)
    }

    fn hypot_libm(self, other: f64) -> f64 {
        libm::hypot(self, other)
    }

    fn cbrt_libm(self) -> f64 {
        libm::cbrt(self)
    }
}

#[cfg(test)]
mod tests {
    use super::Maths;

    #[test]
    fn the_functions_match_the_values_they_are_named_for() {
        let tolerance = 1e-15;

        assert!((1.0_f64.sin_libm() - 0.841_470_984_807_896_5).abs() < tolerance);
        assert!((1.0_f64.cos_libm() - 0.540_302_305_868_139_8).abs() < tolerance);
        assert!((1.0_f64.atan_libm() - std::f64::consts::FRAC_PI_4).abs() < tolerance);
        assert!((1.0_f64.atan2_libm(1.0) - std::f64::consts::FRAC_PI_4).abs() < tolerance);
        assert!((3.0_f64.hypot_libm(4.0) - 5.0).abs() < tolerance);
        assert!((27.0_f64.cbrt_libm() - 3.0).abs() < tolerance);
        assert!((2.0_f64.powf_libm(0.5) - std::f64::consts::SQRT_2).abs() < tolerance);
    }

    #[test]
    fn sin_cos_is_the_pair_of_sin_and_cos() {
        for step in 0..200 {
            let angle = f64::from(step) * 0.037 - 3.5;
            let (sin, cos) = angle.sin_cos_libm();

            assert_eq!(sin, angle.sin_libm());
            assert_eq!(cos, angle.cos_libm());
        }
    }
}
