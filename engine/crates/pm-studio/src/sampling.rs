//! Offline preset controls. Zero bounce limits preserve the legacy total limit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct SamplingPolicy {
    pub edge_multiplier: u32,
    pub perceptual_noise: bool,
    pub diffuse_bounces: u32,
    pub glossy_bounces: u32,
    pub transmission_bounces: u32,
}
impl SamplingPolicy {
    pub const PRODUCTION: Self = Self { perceptual_noise:true, edge_multiplier:2, diffuse_bounces:4, glossy_bounces:8, transmission_bounces:8 };
    pub fn validate(self)->Result<Self,String> {
        if self.edge_multiplier>8 || [self.diffuse_bounces,self.glossy_bounces,self.transmission_bounces].iter().any(|&n|n>32) {
            return Err("Invalid Studio sampling policy".into());
        }
        Ok(self)
    }
    /// At most 23 bits: exactly represented in the reserved float parameter.
    pub(crate) fn packed(self)->u32 {
        self.edge_multiplier | ((self.perceptual_noise as u32)<<22) | (self.diffuse_bounces<<4) | (self.glossy_bounces<<10) | (self.transmission_bounces<<16)
    }
}
