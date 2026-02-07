use nalgebra::Vector3;

/// State of a dynamical system with position and velocity vectors.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub position: Vector3<f64>,
    pub velocity: Vector3<f64>,
}

/// Time derivative of a state: velocity and acceleration vectors.
#[derive(Debug, Clone, PartialEq)]
pub struct StateDerivative {
    pub velocity: Vector3<f64>,
    pub acceleration: Vector3<f64>,
}

/// A dynamical system that can compute state derivatives at a given time.
pub trait DynamicalSystem {
    fn derivatives(&self, t: f64, state: &State) -> StateDerivative;
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::vector;

    #[test]
    fn state_clone_and_debug() {
        let state = State {
            position: vector![1.0, 2.0, 3.0],
            velocity: vector![4.0, 5.0, 6.0],
        };
        let cloned = state.clone();
        assert_eq!(state, cloned);
        // Debug should not panic
        let _debug = format!("{:?}", state);
    }

    #[test]
    fn state_derivative_clone_and_debug() {
        let deriv = StateDerivative {
            velocity: vector![1.0, 0.0, 0.0],
            acceleration: vector![0.0, -9.8, 0.0],
        };
        let cloned = deriv.clone();
        assert_eq!(deriv, cloned);
        let _debug = format!("{:?}", deriv);
    }

    struct UniformMotion {
        constant_velocity: Vector3<f64>,
    }

    impl DynamicalSystem for UniformMotion {
        fn derivatives(&self, _t: f64, _state: &State) -> StateDerivative {
            StateDerivative {
                velocity: self.constant_velocity,
                acceleration: vector![0.0, 0.0, 0.0],
            }
        }
    }

    #[test]
    fn dynamical_system_trait_usage() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let state = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![1.0, 0.0, 0.0],
        };
        let deriv = system.derivatives(0.0, &state);
        assert_eq!(deriv.velocity, vector![1.0, 0.0, 0.0]);
        assert_eq!(deriv.acceleration, vector![0.0, 0.0, 0.0]);
    }
}
