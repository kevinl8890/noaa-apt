//! High-level functions for decoding APT.

use hound;
use png;

use wav;
use dsp::{self, Signal, Rate, Freq};
use err;
use filters;
use context::{Context, Step};
use telemetry;
use misc;


/// Working sample rate, used during demodulation and syncing.
///
/// Should be multiple of the final sample rate. Because the syncing needs that,
/// also that way the final resampling it's just a decimation.
pub const WORK_RATE: u32 = 20800;

/// Final signal sample rate.
///
/// This signal has one sample per pixel.
pub const FINAL_RATE: u32 = 4160;

/// Pixels per image row.
pub const PX_PER_ROW: u32 = 2080;

/// AM carrier frequency in Hz.
pub const CARRIER_FREQ: u32 = 2400;

/// Samples on each image row when at WORK_RATE.
pub const SAMPLES_PER_WORK_ROW: u32 = PX_PER_ROW * WORK_RATE / FINAL_RATE;

/// Load and resample WAV file.
pub fn resample_wav(
    mut context: Context,
    input_filename: &str,
    output_filename: &str,
    output_rate: Rate,
) -> err::Result<()> {

    info!("Reading WAV file");
    let (input_signal, input_spec) = wav::load_wav(input_filename)?;
    let input_rate = Rate::hz(input_spec.sample_rate);

    context.step(Step::signal("input", &input_signal, Some(input_rate)))?;

    info!("Resampling");
    let resampled = dsp::resample(
        &mut context, &input_signal, input_rate, output_rate, Freq::pi_rad(0.1))?;

    if resampled.is_empty() {
        return Err(err::Error::Internal(
            "Got zero samples after resampling, audio file too short or \
            output sampling frequency too low".to_string())
        );
    }

    let writer_spec = hound::WavSpec {
        channels: 1,
        sample_rate: output_rate.get_hz(),
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    info!("Writing WAV to '{}'", output_filename);

    wav::write_wav(output_filename, &resampled, writer_spec)?;

    Ok(())
}

/// Generate sample sync frame.
///
/// Generates sync A frame, a square wave which has a pulse width of 2 pixels,
/// and period of 4 pixels. Only has values 0 and 1.
///
/// Used for cross correlation against the received signal to find the sync
/// frames positions.
fn generate_sync_frame(work_rate: Rate) -> err::Result<Vec<i8>> {

    if work_rate.get_hz() % FINAL_RATE != 0 {
        return Err(err::Error::Internal(
            "work_rate is not multiple of FINAL_RATE".to_string()));
    }

    // Width of pixels at the work_rate.
    let pixel_width = (work_rate.get_hz() / FINAL_RATE) as usize;

    // Width of pulses at work_rate
    let sync_pulse_width = pixel_width * 2;

    // Tried to use iterators, it's horrible

    use std::iter::repeat;

    Ok(
        (
            repeat(-1).take(sync_pulse_width).chain(
            repeat(1).take(sync_pulse_width))
            .cycle().take(7 * 2 * sync_pulse_width)
        ).chain(
        repeat(-1).take(8 * pixel_width)).collect()
    )
}


/// Find sync frame positions.
///
/// Returns list of found sync frames positions.
fn find_sync(context: &mut Context, signal: &Signal) -> err::Result<Vec<usize>> {

    info!("Searching for sync frames");

    let guard = generate_sync_frame(Rate::hz(WORK_RATE))?;

    // list of maximum correlations found: (index, value)
    let mut peaks: Vec<(usize, f32)> = Vec::new();
    peaks.push((0, 0.));

    // Minimum distance between peaks, some arbitrary number smaller but close
    // to the number of samples by line
    let min_distance: usize = SAMPLES_PER_WORK_ROW as usize * 8/10;

    // Save cross-correlation if exporting steps
    let mut correlation = if context.export_steps {
        Vec::with_capacity(signal.len() - guard.len())
    } else {
        Vec::with_capacity(0) // Not going to be used
    };

    for i in 0 .. signal.len() - guard.len() {
        let mut corr: f32 = 0.;
        for j in 0..guard.len() {
            match guard[j] {
                1 => corr += signal[i + j],
                -1 => corr -= signal[i + j],
                _ => unreachable!(),
            }
        }

        if context.export_steps {
            correlation.push(corr);
        }

        // If previous peak is too far, keep it and add this value to the
        // list as a new peak
        if i - peaks.last().unwrap().0 > min_distance {
            peaks.push((i, corr));
        }

        // Else if this value is bigger than the previous maximum, set this
        // one
        else if corr > peaks.last().unwrap().1 {
            peaks.pop();
            peaks.push((i, corr));
        }
    }

    if context.export_steps {
        context.step(Step::signal("sync_correlation", &correlation, None))?;
    }

    info!("Found {} sync frames", peaks.len());

    Ok(peaks.iter().map(|(index, _value)| *index).collect())
}

/// Maps float signal values to `u8`.
///
/// `low` becomes 0 and `high` becomes 255. Values are clamped to prevent `u8`
/// overflow.
fn map(signal: &Signal, low: f32, high: f32) -> Vec<u8> {

    let range = high - low;
    let signal: Vec<u8> = signal.iter()
        .map(|x|
             // Map and clamp between 0 and 255 using min() and max()
             ((x - low) / range * 255.).max(0.).min(255.).round() as u8
        ).collect();

    signal
}

/// Available settings for contrast adjustment.
pub enum Contrast {
    /// From telemetry bands, requires that syncing is enabled.
    Telemetry,

    /// Takes only a given percent of the samples, clamping the rest. Something
    /// like a percentile.
    Percent(f32),

    /// Don't do anything, map the minimum value to zero and the maximum value
    /// to 255
    MinMax,
}

/// Decode APT image from WAV file.
pub fn decode(
    mut context: Context,
    input_filename: &str,
    output_filename: &str,
    contrast_adjustment: Contrast,
    sync: bool,
) -> err::Result<()>{

    info!("Reading WAV file");

    let (signal, input_spec) = wav::load_wav(input_filename)?;
    let input_rate = Rate::hz(input_spec.sample_rate);
    let work_rate = Rate::hz(WORK_RATE);
    let final_rate = Rate::hz(FINAL_RATE);
    context.step(Step::signal("input", &signal, Some(input_rate)))?;

    info!("Resampling to {}", WORK_RATE);

    let filter = filters::LowpassDcRemoval {
        // Cutout frequency of the resampling filter, only the AM spectrum should go
        // through to avoid noise, 2 times the carrier frequency is enough
        cutout: Freq::hz(CARRIER_FREQ as f32, input_rate) * 2.,

        atten: 40.,

        // Width of transition band, we are using a DC removal filter that has a
        // transition band from zero to delta_w. I think that APT signals have
        // nothing below 500Hz.
        delta_w: Freq::hz(500., input_rate)
    };
    let signal = dsp::resample_with_filter(
        &mut context, &signal, input_rate, work_rate, filter)?;

    if signal.len() < 10 * SAMPLES_PER_WORK_ROW as usize {
        return Err(err::Error::Internal(
            "Got less than 10 rows of samples, audio file is too short".to_string()));
    }

    info!("Demodulating");

    let signal = dsp::demodulate(
        &mut context, &signal, Freq::hz(CARRIER_FREQ as f32, work_rate))?;

    info!("Filtering");

    let cutout = Freq::pi_rad(FINAL_RATE as f32 / WORK_RATE as f32);
    let filter = filters::Lowpass {
        cutout,
        atten: 20.,
        delta_w: cutout / 5.
    };
    // mut because on sync the signal is going to be modified
    let mut signal = dsp::filter(&mut context, &signal, filter)?;

    if sync {
        info!("Syncing");

        // Get list of sync frames positions
        let sync_pos = find_sync(&mut context, &signal)?;

        if sync_pos.len() < 5 {
            return Err(err::Error::Internal(
                "Found less than 5 sync frames, audio file is too short or too \
                noisy".to_string())
            );
        }

        // Create new "aligned" vector to SAMPLES_PER_WORK_ROW. Each row starts on
        // a found sync frame position
        let mut aligned: Signal = Vec::new();

        // For each sync position
        for i in 0..sync_pos.len()-1 {
            // Check if there are enough samples left to fill an image row
            if (sync_pos[i] + SAMPLES_PER_WORK_ROW as usize) < signal.len() {

                aligned.extend_from_slice(
                    &signal[sync_pos[i] .. sync_pos[i] + SAMPLES_PER_WORK_ROW as usize]
                );
            }
        }

        signal = aligned;

    } else {
        info!("Not syncing");

        // If we are not syncing send a dummy correlation step
        context.step(Step::signal("sync_correlation", &vec![], Some(work_rate)))?;

        // Crop signal to multiple of SAMPLES_PER_WORK_ROW
        let length = signal.len();
        signal.truncate(length
            / SAMPLES_PER_WORK_ROW as usize // Integer division
            * SAMPLES_PER_WORK_ROW as usize
        );
    }

    context.step(Step::signal("sync_result", &signal, Some(work_rate)))?;

    info!("Resampling to 4160");

    // Resample without filter because we already filtered the signal before
    // syncing
    let signal = dsp::resample_with_filter(
        &mut context, &signal, work_rate, final_rate, filters::NoFilter)?;


    let (low, high) = match contrast_adjustment {
        Contrast::Telemetry => {
            info!("Adjusting contrast from telemetry");
            if !sync {
                warn!("Reading telemetry without syncing, expect horrible results!");
            }

            let telemetry = telemetry::read_telemetry(&mut context, &signal)?;

            let low = telemetry.get_wedge_value(9, None);
            let high = telemetry.get_wedge_value(8, None);

            (low, high)
        },
        Contrast::Percent(p) => {
            info!("Adjusting contrast using {} percent", p * 100.);
            misc::percent(&signal, p)?
        },
        Contrast::MinMax => {
            info!("Mapping values (no contrast adjustment)");
            let low: f32 = *dsp::get_min(&signal)?;
            let high: f32 = *dsp::get_max(&signal)?;

            (low, high)
        }
    };

    let signal = map(&signal, low, high);

    context.step(Step::signal(
            "mapped",
            &signal.iter().map(|x| f32::from(*x)).collect(),
            Some(final_rate)
    ))?;

    info!("Writing PNG to '{}'", output_filename);

    // To use encoder.set()
    use png::HasParameters;

    let path = std::path::Path::new(output_filename);
    let file = std::fs::File::create(path)?;
    let buffer = &mut std::io::BufWriter::new(file);

    let height = signal.len() as u32 / PX_PER_ROW;

    let mut encoder = png::Encoder::new(buffer, PX_PER_ROW, height);
    encoder.set(png::ColorType::Grayscale).set(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;

    writer.write_image_data(&signal[..])?;

    Ok(())
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_sample_sync_frame() {

        assert_eq!(
            vec![-1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
                  1,  1,  1,  1,  1,  1,  1,  1,  1,  1,
                 -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
                  1,  1,  1,  1,  1,  1,  1,  1,  1,  1,
                 -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
                  1,  1,  1,  1,  1,  1,  1,  1,  1,  1,
                 -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
                  1,  1,  1,  1,  1,  1,  1,  1,  1,  1,
                 -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
                  1,  1,  1,  1,  1,  1,  1,  1,  1,  1,
                 -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
                  1,  1,  1,  1,  1,  1,  1,  1,  1,  1,
                 -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
                  1,  1,  1,  1,  1,  1,  1,  1,  1,  1,
                 -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
                 -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
                 -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
                 -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,],
            generate_sync_frame(Rate::hz(FINAL_RATE * 5)).unwrap()
        );

        assert_eq!(
            vec![-1, -1, -1, -1,
                  1,  1,  1,  1,
                 -1, -1, -1, -1,
                  1,  1,  1,  1,
                 -1, -1, -1, -1,
                  1,  1,  1,  1,
                 -1, -1, -1, -1,
                  1,  1,  1,  1,
                 -1, -1, -1, -1,
                  1,  1,  1,  1,
                 -1, -1, -1, -1,
                  1,  1,  1,  1,
                 -1, -1, -1, -1,
                  1,  1,  1,  1,
                 -1, -1, -1, -1,
                 -1, -1, -1, -1,
                 -1, -1, -1, -1,
                 -1, -1, -1, -1,],
            generate_sync_frame(Rate::hz(FINAL_RATE * 2)).unwrap()
        );
    }

    #[test]
    fn test_map() {
        let expected: Vec<u8> = vec![
            0, 0, 0, 0, 1, 2, 50, 120, 200, 255, 255, 255];
        let test_values: Signal = vec![
            -10., -5., -1., 0., 1., 2.4, 50., 120., 199.6, 255., 256., 300.];

        // Shift values somewhere
        let shifted_values: Signal =
            test_values.iter().map(|x| x * 123.123 - 234.234).collect();

        // See where 0 and 255 end up after that
        let low = 0. * 123.123 - 234.234;
        let high = 255. * 123.123 - 234.234;

        assert_eq!(expected, map(&shifted_values, low, high));
    }
}
