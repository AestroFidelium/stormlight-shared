//! Declarative ECS-item macros — the canonical way to declare components and
//! resources in this codebase. Never hand-write `#[derive(Component)] struct …`.
//!
//! ```ignore
//! easy_components!(
//!     derives(Clone, Copy, Debug, Default, Hash, Eq, PartialEq)
//!     IsHero,
//!     Player(u64),   // tuple-struct form also derives Deref / DerefMut
//! );
//! easy_resources!(
//!     derives(Clone, Debug, Default)
//!     FreeUuid(u64),
//! );
//! ```

/// Declare one or more `Component`s sharing a derive set. Tuple-struct forms
/// additionally derive `Deref` / `DerefMut`.
#[macro_export]
macro_rules! easy_components {
    (derives($($d:path),* $(,)?) $($body:tt)+) => {
        $crate::__easy_items!(@trait Component, derives[$($d),*], $($body)+ ,);
    };
}

/// Declare one or more `Resource`s sharing a derive set. Tuple-struct forms
/// additionally derive `Deref` / `DerefMut`.
#[macro_export]
macro_rules! easy_resources {
    (derives($($d:path),* $(,)?) $($body:tt)+) => {
        $crate::__easy_items!(@trait Resource, derives[$($d),*], $($body)+ ,);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __easy_items {
    // tuple-struct form: `Name(ty, …),`
    (@trait $tr:ident, derives[$($d:path),*], $name:ident ( $($ty:ty),+ $(,)? ) , $($rest:tt)*) => {
        #[derive($($d,)* ::bevy::prelude::$tr, ::derive_more::Deref, ::derive_more::DerefMut)]
        pub struct $name( $(pub $ty),+ );
        $crate::__easy_items!(@trait $tr, derives[$($d),*], $($rest)*);
    };
    // unit-struct form: `Name,`
    (@trait $tr:ident, derives[$($d:path),*], $name:ident , $($rest:tt)*) => {
        #[derive($($d,)* ::bevy::prelude::$tr)]
        pub struct $name;
        $crate::__easy_items!(@trait $tr, derives[$($d),*], $($rest)*);
    };
    // terminus
    (@trait $tr:ident, derives[$($d:path),*],) => {};
}
