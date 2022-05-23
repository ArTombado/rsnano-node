mod bootstrap_attempt;
mod bootstrap_initiator;
pub(crate) use bootstrap_attempt::*;
pub(crate) use bootstrap_initiator::*;

mod bootstrap_limits {
    pub(crate) const PULL_COUNT_PER_CHECK: u64 = 8 * 1024;
}

#[derive(FromPrimitive)]
pub(crate) enum BootstrapMode {
    Legacy,
    Lazy,
    WalletLazy,
}
