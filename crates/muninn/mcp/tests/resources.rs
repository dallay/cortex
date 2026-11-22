use muninn_mcp::{get_resource, ResourceProfile};

#[test]
fn get_resource_finds_all_extended_resources() {
    let preferences = get_resource("memory://preferences", ResourceProfile::Extended)
        .expect("preferences resource is available in the extended profile");
    assert_eq!(preferences.template, "memory://preferences");

    let graph_stats = get_resource("memory://graph/stats", ResourceProfile::Extended)
        .expect("graph stats resource is available in the extended profile");
    assert_eq!(graph_stats.template, "memory://graph/stats");
}

#[test]
fn get_resource_hides_extended_resources_in_core_profile() {
    assert!(get_resource("memory://preferences", ResourceProfile::Core).is_none());
    assert!(get_resource("memory://graph/stats", ResourceProfile::Core).is_none());
}
