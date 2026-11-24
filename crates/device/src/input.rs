#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Input {
    One,
    Two,
}

impl Input {
    pub const BOTH: [Input; 2] = [Input::One, Input::Two];
}
