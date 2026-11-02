//! Platform boundary shared by the app and the Linux editing harness.
/// Platform requests contain ids, never JS callbacks. A platform event returns
/// through the owning input queue. In particular sampling must not wait on JS.
pub trait EditingHost {
    type Error;
    fn configure_text(
        &mut self,
        field: &str,
        policy: super::text::TextPolicy,
    ) -> Result<(), Self::Error>;
    fn colour_popover(
        &mut self,
        anchor: &str,
        plan: super::colour::PopoverPlan,
    ) -> Result<(), Self::Error>;
    fn sample_colour(&mut self, request: u64) -> Result<(), Self::Error>;
    fn copy_colour(&mut self, notation: &str) -> Result<(), Self::Error>;
}

/// Linux double: records the same typed requests a main-thread AppKit host takes.
#[derive(Default, Debug)]
pub struct InspectHost {
    pub text: Vec<(String, super::text::TextPolicy)>,
    pub popovers: Vec<(String, super::colour::PopoverPlan)>,
    pub samples: Vec<u64>,
    pub clipboard: Option<String>,
}
impl EditingHost for InspectHost {
    type Error = std::convert::Infallible;
    fn configure_text(
        &mut self,
        field: &str,
        policy: super::text::TextPolicy,
    ) -> Result<(), Self::Error> {
        self.text.push((field.into(), policy));
        Ok(())
    }
    fn colour_popover(
        &mut self,
        anchor: &str,
        plan: super::colour::PopoverPlan,
    ) -> Result<(), Self::Error> {
        self.popovers.push((anchor.into(), plan));
        Ok(())
    }
    fn sample_colour(&mut self, request: u64) -> Result<(), Self::Error> {
        self.samples.push(request);
        Ok(())
    }
    fn copy_colour(&mut self, notation: &str) -> Result<(), Self::Error> {
        self.clipboard = Some(notation.into());
        Ok(())
    }
}
