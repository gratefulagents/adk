use crate::dto::{ModeRouting, ModeTemplateSpec};

pub fn mode_routing_from_template_spec(spec: Option<&ModeTemplateSpec>) -> Option<ModeRouting> {
    let mut routing = spec?.model_routing.as_ref()?.clone();
    // The Go adapter appends into nil slices and allocates only nonempty maps.
    routing.fallback_models = routing.fallback_models.filter(|models| !models.is_empty());
    routing.role_overrides = routing.role_overrides.filter(|roles| !roles.is_empty());
    if let Some(roles) = &mut routing.role_overrides {
        for role in roles.values_mut() {
            role.fallback_models = role
                .fallback_models
                .take()
                .filter(|models| !models.is_empty());
        }
    }
    Some(routing)
}
