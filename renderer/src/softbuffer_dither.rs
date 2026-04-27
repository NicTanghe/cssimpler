use std::sync::OnceLock;

use super::color::{
    is_transparent, pack_softbuffer_channels, u8_to_u10_channel, u10_to_u8_channel, unpack_rgb10,
};

const TILE_WIDTH: usize = 8;
const TILE_HEIGHT: usize = 8;
const TILE_SIZE: usize = TILE_WIDTH * TILE_HEIGHT;
const BEST_CANDIDATE_SAMPLES: usize = 32;

pub(crate) fn to_softbuffer_rgb_blue_noise(pixel: u32, x: usize, y: usize) -> u32 {
    if is_transparent(pixel) {
        return 0;
    }

    let (red10, green10, blue10) = unpack_rgb10(pixel);
    let red = quantize_channel_with_blue_noise(red10, threshold_at(x, y));
    let green =
        quantize_channel_with_blue_noise(green10, threshold_at(x + (TILE_WIDTH / 2), y + 1));
    let blue = quantize_channel_with_blue_noise(blue10, threshold_at(x + 1, y + (TILE_HEIGHT / 2)));
    pack_softbuffer_channels(red, green, blue)
}

fn threshold_at(x: usize, y: usize) -> u8 {
    let tile = blue_noise_tile();
    let x = x % TILE_WIDTH;
    let y = y % TILE_HEIGHT;
    tile[y * TILE_WIDTH + x]
}

fn blue_noise_tile() -> &'static [u8; TILE_SIZE] {
    static TILE: OnceLock<[u8; TILE_SIZE]> = OnceLock::new();
    TILE.get_or_init(generate_blue_noise_tile)
}

fn generate_blue_noise_tile() -> [u8; TILE_SIZE] {
    let mut order = [0_usize; TILE_SIZE];
    let mut used = [false; TILE_SIZE];
    let mut state = 0x9E37_79B9_u32;

    let first = (next_random(&mut state) as usize) % TILE_SIZE;
    order[0] = first;
    used[first] = true;

    for rank in 1..TILE_SIZE {
        let mut best_candidate = None;
        let mut best_distance = 0_u32;

        for _ in 0..BEST_CANDIDATE_SAMPLES {
            let Some(candidate) = random_unused_position(&mut state, &used) else {
                continue;
            };
            let nearest_distance = nearest_selected_distance_sq(candidate, &order[..rank]);
            if best_candidate.is_none() || nearest_distance > best_distance {
                best_candidate = Some(candidate);
                best_distance = nearest_distance;
            }
        }

        let candidate = best_candidate
            .or_else(|| used.iter().position(|is_used| !*is_used))
            .unwrap_or(0);
        order[rank] = candidate;
        used[candidate] = true;
    }

    let mut tile = [0_u8; TILE_SIZE];
    for (rank, &position) in order.iter().enumerate() {
        tile[position] = rank as u8;
    }
    tile
}

fn random_unused_position(state: &mut u32, used: &[bool; TILE_SIZE]) -> Option<usize> {
    for _ in 0..TILE_SIZE {
        let candidate = (next_random(state) as usize) % TILE_SIZE;
        if !used[candidate] {
            return Some(candidate);
        }
    }
    used.iter().position(|is_used| !*is_used)
}

fn nearest_selected_distance_sq(candidate: usize, selected: &[usize]) -> u32 {
    if selected.is_empty() {
        return 0;
    }
    let (candidate_x, candidate_y) = index_to_coords(candidate);
    selected
        .iter()
        .copied()
        .map(|index| {
            let (selected_x, selected_y) = index_to_coords(index);
            toroidal_distance_sq(candidate_x, candidate_y, selected_x, selected_y)
        })
        .min()
        .unwrap_or(0)
}

fn index_to_coords(index: usize) -> (i32, i32) {
    ((index % TILE_WIDTH) as i32, (index / TILE_WIDTH) as i32)
}

fn toroidal_distance_sq(ax: i32, ay: i32, bx: i32, by: i32) -> u32 {
    let dx = toroidal_delta(ax, bx, TILE_WIDTH as i32);
    let dy = toroidal_delta(ay, by, TILE_HEIGHT as i32);
    (dx * dx + dy * dy) as u32
}

fn toroidal_delta(a: i32, b: i32, period: i32) -> i32 {
    let delta = (a - b).abs();
    delta.min(period - delta)
}

fn next_random(state: &mut u32) -> u32 {
    let mut value = *state;
    value ^= value << 13;
    value ^= value >> 17;
    value ^= value << 5;
    *state = value;
    value
}

fn quantize_channel_with_blue_noise(channel10: u16, threshold: u8) -> u8 {
    let nearest = u10_to_u8_channel(channel10);
    let anchor = u8_to_u10_channel(nearest);
    if channel10 == anchor {
        return nearest;
    }

    let (lower8, upper8) = if channel10 > anchor {
        (nearest, nearest.saturating_add(1))
    } else {
        (nearest.saturating_sub(1), nearest)
    };

    if lower8 == upper8 {
        return lower8;
    }

    let lower_anchor = u32::from(u8_to_u10_channel(lower8));
    let upper_anchor = u32::from(u8_to_u10_channel(upper8));
    let span = upper_anchor.saturating_sub(lower_anchor);
    if span == 0 {
        return lower8;
    }

    let position = u32::from(channel10).saturating_sub(lower_anchor).min(span);
    let choose_upper = position.saturating_mul(TILE_SIZE as u32) > u32::from(threshold) * span;
    if choose_upper { upper8 } else { lower8 }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use cssimpler_core::Color;

    use super::{blue_noise_tile, quantize_channel_with_blue_noise, to_softbuffer_rgb_blue_noise};
    use crate::color::u8_to_u10_channel;
    use crate::{pack_rgb, pack_softbuffer_rgb};

    #[test]
    fn blue_noise_tile_is_a_full_permutation() {
        let tile = blue_noise_tile();
        let unique = tile.iter().copied().collect::<HashSet<_>>();
        assert_eq!(unique.len(), 64);
        assert_eq!(unique.iter().copied().min(), Some(0));
        assert_eq!(unique.iter().copied().max(), Some(63));
    }

    #[test]
    fn exact_8bit_anchor_channels_do_not_dither() {
        for channel in 0_u16..=255 {
            let channel10 = u8_to_u10_channel(channel as u8);
            for threshold in 0_u8..64 {
                assert_eq!(
                    quantize_channel_with_blue_noise(channel10, threshold),
                    channel as u8
                );
            }
        }
    }

    #[test]
    fn exact_8bit_colors_stay_stable_after_present_conversion() {
        let color = Color::rgb(45, 117, 240);
        let packed = pack_rgb(color);
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    to_softbuffer_rgb_blue_noise(packed, x, y),
                    pack_softbuffer_rgb(color)
                );
            }
        }
    }
}
