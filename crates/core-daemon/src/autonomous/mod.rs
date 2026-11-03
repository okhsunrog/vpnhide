//! Core-owned autonomous fixture orchestration. Interface-only evidence is a
//! scoped result until the complete supported diagnostic plan is measured.

pub mod native_fixture;

pub mod coordinator;

pub(crate) mod android_worker;

mod framework_fixture;

mod framework_run;

pub mod native_raw;
