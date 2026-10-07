//! Sensor readings for the recording, shared by the controlled and the
//! controller-less attitude paths.

use arika::epoch::Epoch;
use nalgebra::Vector3;
use orts::sensor::{Magnetometer, OnboardMagneticSources};
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
    /// in between. `None` about a body with no field model.
    pub geomagnetic_field_body: Option<Vector3<f64>>,
}

/// The epoch the sensors are evaluated at for sim time `t`: the run's epoch
/// moved on by `t`, or J2000 for a run without one.
pub fn sample_epoch(epoch: Option<&Epoch>, t: f64) -> Epoch {
    epoch.map(|e| e.add_si_seconds(t)).unwrap_or(Epoch::j2000())
}

/// An ideal, uncoupled magnetometer on the field model the sensors use, which
/// reads the geomagnetic field itself; `None` about a body with no field
/// model, where there is no field to record.
pub fn geomagnetic_truth_for(body: arika::body::KnownBody) -> Option<Magnetometer> {
    orts::magnetic::field_is_modelled(body)
        .then(|| Magnetometer::new(orts::magnetic::igrf_field_for_body(body)))
}

/// Read `magnetometers` and `truth` (from [`geomagnetic_truth_for`]) at sim
/// time `t`, for a state at `t`, with the onboard sources as they are at `t`.
pub fn read_magnetometers(
    magnetometers: &mut [Magnetometer],
    truth: Option<&mut Magnetometer>,
    t: f64,
    state: &SpacecraftState,
    epoch: Option<&Epoch>,
    sources: &OnboardMagneticSources,
) -> MagnetometerTelemetry {
    let epoch = sample_epoch(epoch, t);
    let geomagnetic_field_body = truth.map(|truth| {
        truth
            .measure(t, state, &epoch, &OnboardMagneticSources::none())
            .into_inner()
            .into_inner()
    });
    let readings = magnetometers
        .iter_mut()
        .map(|m| {
            m.measure(t, state, &epoch, sources)
                .into_inner()
                .into_inner()
        })
        .collect();
    MagnetometerTelemetry {
        readings,
        geomagnetic_field_body,
    }
}

/// The magnetometers and the geomagnetic field of a satellite without a
/// controller, read only for the recording.
///
/// Nothing reads them during the run, but what they would read is still a
/// result: the geomagnetic field through the sensor's noise, along an attitude
/// that no controller steers. The field itself is recorded whether or not a
/// magnetometer is mounted.
pub struct MagnetometerProbe {
    magnetometers: Vec<Magnetometer>,
    truth: Option<Magnetometer>,
    /// The onboard sources throughout the run: an MTQ without a controller is
    /// never commanded, so its rods stay off.
    sources: OnboardMagneticSources,
}

impl MagnetometerProbe {
    /// The probe for `spec` about `body`, or `None` when there is nothing to
    /// record: no magnetometer and no field model.
    pub fn for_spec(
        spec: &SatelliteSpec,
        body: arika::body::KnownBody,
    ) -> Result<Option<Self>, String> {
        let bundle =
            crate::sim::controlled::build_sensor_bundle(spec.sensors.as_deref(), body, &spec.id)?;
        let truth = geomagnetic_truth_for(body);
        if bundle.magnetometers.is_empty() && truth.is_none() {
            return Ok(None);
        }
        let sources = match &spec.mtq_config {
            Some(crate::config::MtqConfig::ThreeAxis { max_moment }) => {
                let rods = orts::spacecraft::MtqAssemblyCore::three_axis(*max_moment).num_mtqs();
                OnboardMagneticSources::none().with_mtq_rod_moments(vec![0.0; rods])
            }
            None => OnboardMagneticSources::none(),
        };
        Ok(Some(Self {
            magnetometers: bundle.magnetometers,
            truth,
            sources,
        }))
    }

    /// The readings at sim time `t` for a state at `t`.
    pub fn read(
        &mut self,
        t: f64,
        state: &SpacecraftState,
        epoch: Option<&Epoch>,
    ) -> MagnetometerTelemetry {
        read_magnetometers(
            &mut self.magnetometers,
            self.truth.as_mut(),
            t,
            state,
            epoch,
            &self.sources,
        )
    }
}
