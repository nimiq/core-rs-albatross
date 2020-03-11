use algebra::bls12_377::{Fq, Fq2};
use algebra::curves::models::short_weierstrass_jacobian::GroupAffine;
use algebra::sw6::Fr;
use algebra::{One, SWModelParameters, Zero};

pub trait Input<I>
where
    Self: Sized,
{
    fn append_to_inputs(&self, inputs: &mut Vec<I>);
}

// Non-generic implementations
impl Input<Fr> for Fq {
    fn append_to_inputs(&self, inputs: &mut Vec<Fr>) {
        inputs.push(*self);
    }
}

impl Input<Fr> for bool {
    fn append_to_inputs(&self, inputs: &mut Vec<Fr>) {
        if *self {
            inputs.push(Fr::one());
        } else {
            inputs.push(Fr::zero());
        }
    }
}

impl Input<Fr> for Fq2 {
    fn append_to_inputs(&self, inputs: &mut Vec<Fr>) {
        Input::append_to_inputs(&self.c0, inputs);
        Input::append_to_inputs(&self.c1, inputs);
    }
}

impl<P: SWModelParameters> Input<Fr> for GroupAffine<P>
where
    P::BaseField: Input<Fr>,
{
    fn append_to_inputs(&self, inputs: &mut Vec<Fr>) {
        Input::append_to_inputs(&self.x, inputs);
        Input::append_to_inputs(&self.y, inputs);
        Input::append_to_inputs(&self.infinity, inputs);
    }
}
