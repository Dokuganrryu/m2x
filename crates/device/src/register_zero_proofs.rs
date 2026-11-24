#[cfg(not(feature = "register-zero-protocol"))]
#[doc = include_str!("../compile_fail/register_zero_absent_from_normal_build.md")]
pub struct RegisterZeroIsAbsentFromTheNormalBuild;

#[cfg(feature = "register-zero-protocol")]
#[doc = include_str!("../compile_fail/register_zero_protocol_build.md")]
pub struct RegisterZeroWriteIsSealedToTheFactoryValue;
