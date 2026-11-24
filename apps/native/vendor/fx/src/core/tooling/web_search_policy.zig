const std = @import("std");
const web_search_contract = @import("web_search_contract.zig");

pub const SearchBackendId = web_search_contract.SearchBackendId;
pub const BackendFeatureMode = web_search_contract.BackendFeatureMode;
pub const BackendCapabilities = web_search_contract.BackendCapabilities;

pub const BackendPolicy = struct {
    id: SearchBackendId,
    features: BackendCapabilities,
};

pub const InputFeatures = struct {
    has_allowed_domains: bool = false,
    has_blocked_domains: bool = false,
};

pub const WebSearchPolicy = struct {
    max_uses: u8 = 8,
    max_results: u8 = 10,
    max_output_tokens: u32 = 4096,
    max_output_chars: usize = 100_000,
    max_input_bytes: usize = 16 * 1024,
    timeout_ms: u32 = 30_000,
    strict_filters: bool = true,
    preferred_backends: []const SearchBackendId = &.{},
    backend_policies: []const BackendPolicy = &.{},
};

pub fn hasAdmittedBackendPolicy(backends: []const BackendPolicy) bool {
    for (backends) |backend| {
        if (backendPolicyIsAdmitted(backend)) return true;
    }
    return false;
}

pub fn selectBackend(policy: WebSearchPolicy, input: InputFeatures) ?BackendPolicy {
    // Relaxed filter behavior needs a separate explicit opt-in contract.
    if (!policy.strict_filters and (input.has_allowed_domains or input.has_blocked_domains)) return null;
    for (policy.preferred_backends) |id| {
        const backend = backendPolicyForId(policy.backend_policies, id) orelse continue;
        if (backendCanSatisfy(backend, input)) return backend;
    }
    return null;
}

pub fn backendCanSatisfy(backend: BackendPolicy, input: InputFeatures) bool {
    if (!backendPolicyIsAdmitted(backend)) return false;
    if (input.has_allowed_domains and !strictFilterIsSupported(backend.features.allowed_domains)) return false;
    if (input.has_blocked_domains and !strictFilterIsSupported(backend.features.blocked_domains)) return false;
    return true;
}

fn backendPolicyForId(backends: []const BackendPolicy, id: SearchBackendId) ?BackendPolicy {
    for (backends) |backend| {
        if (backend.id.eql(id)) return backend;
    }
    return null;
}

fn backendPolicyIsAdmitted(backend: BackendPolicy) bool {
    const features = backend.features;
    return features.max_uses != .unsupported and
        features.ordered_sources and
        features.usage and
        features.terminal_incomplete and
        features.timeout and
        features.cancellation and
        (features.result_bounds == .pass_through or features.result_bounds == .post_filter);
}

fn strictFilterIsSupported(support: BackendFeatureMode) bool {
    // Post-filtering is not admitted until the runtime implements it.
    return support == .pass_through;
}

fn admittedFeatures() BackendCapabilities {
    return .{
        .max_uses = .best_effort,
        .allowed_domains = .pass_through,
        .blocked_domains = .pass_through,
        .ordered_sources = true,
        .usage = true,
        .terminal_incomplete = true,
        .timeout = true,
        .cancellation = true,
        .result_bounds = .post_filter,
    };
}

const test_primary_backend = SearchBackendId{ .value = "test.primary" };
const test_secondary_backend = SearchBackendId{ .value = "test.secondary" };

test "core policy has no implicit provider backend" {
    try std.testing.expect(selectBackend(.{}, .{}) == null);
}

test "backend feature support is explicit and strict filters cannot silently weaken" {
    var features = admittedFeatures();
    features.allowed_domains = .unsupported;
    const unsupported = BackendPolicy{
        .id = test_primary_backend,
        .features = features,
    };

    try std.testing.expect(!backendCanSatisfy(unsupported, .{ .has_allowed_domains = true }));
    try std.testing.expect(backendCanSatisfy(unsupported, .{ .has_blocked_domains = true }));
}

test "backend selection skips backend that cannot satisfy strict filters" {
    var unsupported_features = admittedFeatures();
    unsupported_features.allowed_domains = .unsupported;
    const backends = [_]BackendPolicy{
        .{
            .id = test_primary_backend,
            .features = unsupported_features,
        },
        .{
            .id = test_secondary_backend,
            .features = admittedFeatures(),
        },
    };
    const selected = selectBackend(.{
        .preferred_backends = &.{
            test_primary_backend,
            test_secondary_backend,
        },
        .backend_policies = &backends,
    }, .{
        .has_allowed_domains = true,
    }) orelse return error.TestExpectedEqual;

    try std.testing.expect(test_secondary_backend.eql(selected.id));
}

test "backend selection rejects unsupported strict filters before search" {
    var features = admittedFeatures();
    features.allowed_domains = .unsupported;
    const backends = [_]BackendPolicy{.{
        .id = test_primary_backend,
        .features = features,
    }};

    try std.testing.expect(selectBackend(.{
        .preferred_backends = &.{test_primary_backend},
        .backend_policies = &backends,
    }, .{
        .has_allowed_domains = true,
    }) == null);
}

test "post filter marker cannot silently admit strict domain filtering" {
    var features = admittedFeatures();
    features.blocked_domains = .post_filter;

    try std.testing.expect(!backendCanSatisfy(.{
        .id = test_secondary_backend,
        .features = features,
    }, .{
        .has_blocked_domains = true,
    }));
}
