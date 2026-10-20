pub mod boat;
pub mod minecart;
#[allow(clippy::module_inception)]
pub mod vehicle;

#[cfg(test)]
mod b1020d_g06_admission_tests;
#[cfg(test)]
mod b1020d_g06_drop_tests;
#[cfg(test)]
mod b1020d_g06_lava_tests;
#[cfg(test)]
mod b1020d_g06_retirement_tests;
#[cfg(test)]
mod b1020d_g06_test_support;
