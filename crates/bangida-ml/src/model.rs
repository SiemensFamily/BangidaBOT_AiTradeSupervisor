use tracing::debug;

use crate::features::FeatureVector;

/// Placeholder ML model for signal strength adjustment.
///
/// In Phase 3+, this will load an ONNX model and run real inference.
/// For now it returns a neutral prediction (no adjustment).
pub struct MlModel {
    /// Model name for logging.
    name: String,
    /// Whether the model is loaded and ready for inference.
    loaded: bool,
}

impl MlModel {
    /// Create a new placeholder model.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            loaded: false,
        }
    }

    /// Simulate loading a model. In production this would load an ONNX file.
    pub fn load(&mut self, _path: &str) -> anyhow::Result<()> {
        // Placeholder: in production, load ONNX runtime session here
        self.loaded = true;
        debug!(model = %self.name, "model loaded (placeholder)");
        Ok(())
    }

    /// Predict a signal strength modifier from features.
    ///
    /// Returns a value in [-1.0, 1.0]:
    /// - Positive values amplify the signal (model agrees with direction)
    /// - Negative values dampen or reverse the signal
    /// - 0.0 means no ML adjustment (neutral)
    ///
    /// Currently always returns 0.0 (no ML adjustment).
    pub fn predict(&self, features: &FeatureVector) -> f64 {
        debug!(
            model = %self.name,
            loaded = self.loaded,
            "predict called (placeholder, returning 0.0)"
        );
        let _ = features; // Will be used when real inference is implemented
        0.0
    }

    /// Whether the model is loaded.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Model name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Default for MlModel {
    fn default() -> Self {
        Self::new("default")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_placeholder_returns_zero() {
        let model = MlModel::new("test");
        let fv = FeatureVector::extract(
            0.5, 10.0, 50000.0, 55.0, 0.1, 100.0, 50.0, 30.0,
            0.0001, 49990.0, 50010.0, 50000.0, 51000.0, 49000.0, 50000.0, 50005.0,
        );
        assert_eq!(model.predict(&fv), 0.0);
    }

    #[test]
    fn test_load() {
        let mut model = MlModel::new("test");
        assert!(!model.is_loaded());
        model.load("/fake/path").unwrap();
        assert!(model.is_loaded());
    }

    #[test]
    fn test_default() {
        let model = MlModel::default();
        assert_eq!(model.name(), "default");
        assert!(!model.is_loaded());
    }
}
