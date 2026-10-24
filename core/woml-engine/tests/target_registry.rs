use woml_engine::{parse_target_registry, TargetRegistryError};

const REVIEWED: &str =
  include_str!("../../../woml/tests/fixtures/remote-execution-targets/target-registry.v1.json");

#[test]
fn registry_v1_is_normalized_and_digest_stable() {
  let first = parse_target_registry(REVIEWED.as_bytes()).unwrap();
  let second = parse_target_registry(REVIEWED.as_bytes()).unwrap();
  assert_eq!(first.digest, second.digest);
  assert_eq!(
    first.digest,
    "sha256:d7f8a876583ef92e40aabaca6ed0a78f4189bbeaa374caeca1b5ed4ae7039f93"
  );
  assert_eq!(first.registry.targets[0].id, "production-private");
  assert_eq!(
    first.registry.targets[0].required_capabilities,
    ["runtime.script", "services.artifacts", "services.process"]
  );
}

#[test]
fn registry_v1_rejects_unknown_duplicate_reserved_and_future_contracts() {
  let unknown = REVIEWED.replace("\"version\": 1,", "\"version\": 1, \"unknown\": true,");
  assert!(matches!(
    parse_target_registry(unknown.as_bytes()),
    Err(TargetRegistryError::Invalid(_))
  ));
  let duplicate = REVIEWED.replace("\"version\": 1,", "\"version\": 1, \"version\": 1,");
  assert!(matches!(
    parse_target_registry(duplicate.as_bytes()),
    Err(TargetRegistryError::Invalid(_))
  ));
  let reserved = REVIEWED.replace("production-private", "local");
  assert!(matches!(
    parse_target_registry(reserved.as_bytes()),
    Err(TargetRegistryError::Invalid(_))
  ));
  let future = REVIEWED.replace("\"version\": 1", "\"version\": 2");
  assert!(matches!(
    parse_target_registry(future.as_bytes()),
    Err(TargetRegistryError::Incompatible(_))
  ));
}
