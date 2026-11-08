//! **Function-hosted modules**: a service module the environment reaches as a function (a Neon
//! Function on the branch it is bound to) instead of running it.
//!
//! A module that can run as one says so with [`Provides::function`] (its slug). When the
//! selection's [`hosted`](super::Selection::hosted) entry for it is a
//! [`HostedConnection::Function`], the planner plans against a catalog in which the module is
//! wrapped in a [`FunctionHosted`]: the same id, services, knobs and client environment, so
//! everything that refers to it (a `stack.has("unity-catalog-oss")` check, a client value naming
//! its endpoint) still does, but hosted like any [hosted provider](super::hosted). Nothing of
//! it renders: no compose service, no helpers, no resource demands (the function brings its own
//! storage), and no module it requires. Its services are reached at the function's URL.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

use super::{PlanError, Selection};
use crate::catalog::Catalog;
use crate::catalog::deploy::{DeployPhase, DeployStepSpec};
use crate::catalog::module::{
    ClientValue, HelperService, Knob, KnobRejection, Module, ModuleId, Provides, RenderSpec,
    ResolvedKnobs, ResourceDemand, StackFacts,
};
use crate::model::role::ServiceSpec;
use crate::plan::hosted::HostedKind;
use crate::target::TargetId;

/// The catalog to plan `selection` against: `catalog` with every module the selection hosts
/// as a function wrapped in a [`FunctionHosted`], or `None` when it hosts none.
///
/// A function entry for a module that can't run as one (no [`Provides::function`]), or that
/// isn't in the catalog, is a [`PlanError::InvalidHosted`].
pub(super) fn host_functions(
    selection: &Selection,
    catalog: &Catalog,
) -> Result<Option<Catalog>, PlanError> {
    let mut hosted = None;
    for (id, resource) in &selection.hosted {
        if resource.connection.kind() != HostedKind::Function {
            continue;
        }
        let invalid = |reason: &str| PlanError::InvalidHosted {
            module: id.clone(),
            reason: reason.into(),
        };
        let module = catalog
            .get(id)
            .ok_or_else(|| invalid("no such module in the catalog"))?;
        if module.provides().function.is_none() {
            return Err(invalid("it can't run as a function"));
        }
        hosted
            .get_or_insert_with(|| catalog.clone())
            .insert(Arc::new(FunctionHosted::new(Arc::clone(module))));
    }
    Ok(hosted)
}

/// A service module hosted as a function (see the [module docs](self)).
pub(crate) struct FunctionHosted {
    inner: Arc<dyn Module>,
    provides: Provides,
}

impl FunctionHosted {
    fn new(inner: Arc<dyn Module>) -> Self {
        let own = inner.provides();
        let provides = Provides {
            client_env: own.client_env.clone(),
            python: own.python.clone(),
            function: own.function.clone(),
            function_env: own.function_env.clone(),
            function_secrets: own.function_secrets.clone(),
            // Still the API's server: consumers reach it at the function's URL.
            api: own.api.clone(),
            hosted: Some(HostedKind::Function),
            deploy: function_steps(),
            ..Provides::default()
        };
        FunctionHosted { inner, provides }
    }
}

/// The step that deploys the function on the branch.
fn function_steps() -> Vec<DeployStepSpec> {
    vec![DeployStepSpec::new(
        "function",
        DeployPhase::Provision,
        "Deploy the function",
        include_str!("../../templates/hosted/function.deploy.md.jinja"),
    )]
}

impl Module for FunctionHosted {
    fn id(&self) -> &ModuleId {
        self.inner.id()
    }
    fn model(&self) -> Option<&str> {
        self.inner.model()
    }
    fn display_name(&self) -> Option<&str> {
        self.inner.display_name()
    }
    fn summary(&self) -> Option<&str> {
        self.inner.summary()
    }
    fn category(&self) -> Option<&str> {
        self.inner.category()
    }
    fn provider_of(&self) -> Option<&str> {
        self.inner.provider_of()
    }
    // What it would run beside (the gateway it registers with) and the resources it would
    // demand are the function's own business.
    fn requires(&self) -> &[ModuleId] {
        &[]
    }
    fn conflicts_with(&self) -> &[ModuleId] {
        self.inner.conflicts_with()
    }
    fn needs(&self) -> &[ResourceDemand] {
        &[]
    }
    fn needs_in(&self, _knobs: &ResolvedKnobs) -> Cow<'_, [ResourceDemand]> {
        Cow::Borrowed(&[])
    }
    fn knobs(&self) -> &[Knob] {
        self.inner.knobs()
    }
    fn check_knobs(&self, knobs: &ResolvedKnobs) -> Result<(), KnobRejection> {
        self.inner.check_knobs(knobs)
    }
    fn provides(&self) -> &Provides {
        &self.provides
    }
    fn render_for(&self, _target: &TargetId) -> Option<&RenderSpec> {
        None
    }
    fn services(&self, knobs: &ResolvedKnobs) -> Vec<ServiceSpec> {
        self.inner.services(knobs)
    }
    fn services_in(&self, knobs: &ResolvedKnobs, stack: &StackFacts) -> Vec<ServiceSpec> {
        self.inner.services_in(knobs, stack)
    }
    fn client_env_in(
        &self,
        knobs: &ResolvedKnobs,
        stack: &StackFacts,
    ) -> Cow<'_, BTreeMap<String, ClientValue>> {
        self.inner.client_env_in(knobs, stack)
    }
    fn integrates_with_in(&self, knobs: &ResolvedKnobs, stack: &StackFacts) -> Vec<ModuleId> {
        self.inner.integrates_with_in(knobs, stack)
    }
    fn helpers_in(&self, _knobs: &ResolvedKnobs) -> Cow<'_, [HelperService]> {
        Cow::Borrowed(&[])
    }
}
