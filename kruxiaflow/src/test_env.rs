//! Test support: environment changes that undo themselves

use std::ffi::OsString;

/// Puts an environment variable back the way the guard found it when the
/// guard drops, also when the test panics.
///
/// Every test in a binary shares one process environment, and
/// `#[sqlx::test]` reads `DATABASE_URL` as each test starts. The guard keeps
/// a change from outliving its test; `#[serial]`, on both the test that
/// changes a variable and the tests that read it, keeps them from
/// overlapping.
pub struct EnvVarGuard {
    name: &'static str,
    prior: Option<OsString>,
}

impl EnvVarGuard {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            prior: std::env::var_os(name),
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.prior {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }
}
