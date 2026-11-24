use dsp::{Attenuation, Frame, StreamSample};

const fn linear_fade_gains<const N: usize>() -> [Attenuation; N] {
    let mut gains = [Attenuation::from_ratio(0, 1); N];
    let mut index = 0;
    while index < N {
        gains[index] = Attenuation::from_ratio((N - index) as u32, N as u32);
        index += 1;
    }
    gains
}

struct FadeGains<const N: usize>;

impl<const N: usize> FadeGains<N> {
    const LINEAR: [Attenuation; N] = linear_fade_gains::<N>();
}

pub(crate) fn fade_out<const N: usize>(block: &mut [Frame<StreamSample>; N]) {
    for (frame, &gain) in block.iter_mut().zip(FadeGains::<N>::LINEAR.iter()) {
        for sample in frame.iter_mut() {
            *sample = gain.attenuate(*sample);
        }
    }
}
