//! Sensor readings for the recording, shared by the controlled and the
//! controller-less attitude paths.

use arika::epoch::Epoch;
use nalgebra::Vector3;
use orts::sensor::Magnetometer;
use orts::spacecraft::SpacecraftState;

use crate::satellite::SatelliteSpec;

/// The magnetometers' readings at one sim time and the geomagnetic field
/// there, for telemetry.
///
/// The sensors evaluated again at the output time rather than a reading kept
/// from a tick: the noise is a function of the sample time, so at a tick this
/// is what the controller received, and anywhere else it is the reading the
/// sensors would have given (DESIGN.md "センサの読み値も出力サンプル時刻で評価し直す").
#[derive(Debug, Clone, PartialEq)]
pub struct MagnetometerTelemetry {
    /// Each magnetometer's reading [T, body frame], in bundle order.
    pub readings: Vec<Vector3<f64>>,
    /// The geomagnetic field at the spacecraft [T, body frame], with no sensor
    /// in between.
    pub geomagnetic_field_body: Vector3<f64>,
}

/// The epoch the sensors are evaluated at for sim time `t`: the run's epoch
/// moved on by `t`, or J2000 for a run without one.
pub fn sample_epoch(epoch: Option<&Epoch>, t: f64) -> Epoch {
    epoch.map(|e| e.add_si_seconds(t)).unwrap_or(Epoch::j2000())
}

/// Read `magnetometers` and `truth` (an ideal, uncoupled magnetometer on the
/// same field model) at sim time `t`, for a state at `t`.
pub fn read_magnetometers(
    magnetometers: &mut [Magnetometer],
    truth: &mut Magnetometer,
    t: f64,
    state: &SpacecraftState,
    epoch: Option<&Epoch>,
) -> MagnetometerTelemetry {
    let epoch = sample_epoch(epoch, t);
    let geomagnetic_field_body = truth.measure(t, state, &epoch).into_inner().into_inner();
    let readings = magnetometers
        .iter_mut()
        .map(|m| m.measure(t, state, &epoch).into_inner().into_inner())
        .collect();
    MagnetometerTelemetry {
        readings,
        geomagnetic_field_body,
    }
}

/// The magnetometers of a satellite without a controller, read only for the
/// recording.
///
/// Nothing reads them during the run, but what they would read is still a
/// result: the geomagnetic field through the sensor's noise and its residual
/// field, along an attitude that no controller steers.
pub struct MagnetometerProbe {
    magnetometers: Vec<Magnetometer>,
    truth: Magnetometer,
}

impl MagnetometerProbe {
    /// The probe for `spec`'s configured magnetometers about `body`, or
    /// `None` when it has none.
    pub fn for_spec(
        spec: &SatelliteSpec,
        body: arika::body::KnownBody,
    ) -> Result<Option<Self>, String> {
        let bundle = crate::sim::controlled::build_sensor_bundle(
            spec.sensor_choices.as_deref(),
            body,
            &spec.id,
        )?;
        if bundle.magnetometers.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self {
            magnetometers: bundle.magnetometers,
            truth: Magnetometer::new(orts::magnetic::igrf_field_for_body(body)),
        }))
    }

    /// The readings at sim time `t` for a state at `t`.
    pub fn read(
        &mut self,
        t: f64,
        state: &SpacecraftState,
        epoch: Option<&Epoch>,
    ) -> MagnetometerTelemetry {
        read_magnetometers(&mut self.magnetometers, &mut self.truth, t, state, epoch)
    }
}
