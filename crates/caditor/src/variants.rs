macro_rules! all_variants {
    ($ty:ident: $($variant:ident),+ $(,)?) => {
        impl $ty {
            pub const ALL: [Self; [$(stringify!($variant)),+].len()] = [$(Self::$variant),+];
        }

        const _: fn($ty) = |value| match value {
            $($ty::$variant)|+ => {}
        };
    };
}

pub(crate) use all_variants;
