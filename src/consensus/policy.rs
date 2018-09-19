use bigdecimal::BigDecimal;
use consensus::base::primitive::coin::Coin;
use num_bigint::BigInt;
use num_traits::pow;

lazy_static! {
    pub static ref BLOCK_TARGET_MAX: BigDecimal = {
        BigDecimal::new(pow(BigInt::from(2), 240), 0)
    };
}

pub fn block_reward_at(block_height: u32) -> Coin {
    // TODO
    return Coin(42);
}
