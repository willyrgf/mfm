use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use mfm_canonical::sha256_digest_bytes;

type Digest = [u8; 32];

const SUBJECT_SCHEMA: &str = "mfm.test.balance-subject.v1";
const RESPONSE_SCHEMA: &str = "mfm.test.balance-response.v1";
const OPAQUE_SCHEMA: &str = "mfm.test.portable-object.v1";

const MAX_SCAN_STEP_PUBLICATIONS: usize = 4_096;
const MAX_SCAN_STEP_FACTS: usize = 8_192;
const MAX_RESULT_LIMIT: usize = 128;

const MAX_CLOSURE_STEP_SOURCES: usize = 256;
const MAX_CLOSURE_STEP_OBJECT_REFS: usize = 512;
const MAX_CLOSURE_STEP_BYTES: usize = 256 * 1_024;

fn digest(domain: &[u8], parts: &[&[u8]]) -> Digest {
    let mut preimage = Vec::with_capacity(
        domain.len()
            + parts
                .iter()
                .map(|part| size_of::<u64>() + part.len())
                .sum::<usize>(),
    );
    preimage.extend_from_slice(domain);
    for part in parts {
        preimage.extend_from_slice(&(part.len() as u64).to_be_bytes());
        preimage.extend_from_slice(part);
    }
    *sha256_digest_bytes(&preimage).as_bytes()
}

fn append_sized(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
    bytes.extend_from_slice(value);
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ValueRef {
    object_id: Digest,
    content_digest: Digest,
    evidence_digest: Digest,
    byte_length: usize,
    schema_id: String,
}

impl ValueRef {
    fn for_bytes(schema_id: &str, bytes: &[u8]) -> Self {
        let byte_length = bytes.len();
        let length = (byte_length as u64).to_be_bytes();
        let content_digest = digest(b"mfm.test.content.v1", &[bytes]);
        let object_id = digest(
            b"mfm.test.object.v1",
            &[schema_id.as_bytes(), &content_digest, &length],
        );
        let evidence_digest = digest(
            b"mfm.test.object-evidence.v1",
            &[schema_id.as_bytes(), &object_id, &content_digest, &length],
        );
        Self {
            object_id,
            content_digest,
            evidence_digest,
            byte_length,
            schema_id: schema_id.to_owned(),
        }
    }

    fn canonical_metadata_bytes(&self) -> usize {
        // Three fixed-width digests, byte length, and a length-prefixed schema id.
        3 * 32 + size_of::<u64>() + size_of::<u64>() + self.schema_id.len()
    }

    fn append_prototype_bytes(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.object_id);
        bytes.extend_from_slice(&self.content_digest);
        bytes.extend_from_slice(&self.evidence_digest);
        bytes.extend_from_slice(&(self.byte_length as u64).to_be_bytes());
        append_sized(bytes, self.schema_id.as_bytes());
    }
}

#[derive(Debug, Default, Clone)]
struct RetainedObjectStore {
    objects: BTreeMap<Digest, Vec<u8>>,
}

impl RetainedObjectStore {
    fn retain(&mut self, schema_id: &str, bytes: Vec<u8>) -> ValueRef {
        let value_ref = ValueRef::for_bytes(schema_id, &bytes);
        self.objects.insert(value_ref.object_id, bytes);
        value_ref
    }

    fn remove(&mut self, object_id: &Digest) {
        self.objects.remove(object_id);
    }

    fn replace_bytes(&mut self, object_id: Digest, bytes: Vec<u8>) {
        self.objects.insert(object_id, bytes);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RetainedObjectError {
    MissingObject,
    WrongSchema {
        expected: &'static str,
        actual: String,
    },
    LengthMismatch,
    ContentDigestMismatch,
    ObjectIdMismatch,
    EvidenceDigestMismatch,
    DecodeFailure {
        schema_id: &'static str,
    },
}

trait RetainedTypedValue: Sized {
    const SCHEMA_ID: &'static str;

    fn decode(bytes: &[u8]) -> Option<Self>;
}

struct RetainedObjectMaterializer<'a> {
    store: &'a RetainedObjectStore,
}

impl<'a> RetainedObjectMaterializer<'a> {
    fn new(store: &'a RetainedObjectStore) -> Self {
        Self { store }
    }

    fn verified_bytes(&self, expected_ref: &ValueRef) -> Result<&'a [u8], RetainedObjectError> {
        let bytes = self
            .store
            .objects
            .get(&expected_ref.object_id)
            .ok_or(RetainedObjectError::MissingObject)?;
        if bytes.len() != expected_ref.byte_length {
            return Err(RetainedObjectError::LengthMismatch);
        }

        let rederived = ValueRef::for_bytes(&expected_ref.schema_id, bytes);
        if rederived.content_digest != expected_ref.content_digest {
            return Err(RetainedObjectError::ContentDigestMismatch);
        }
        if rederived.object_id != expected_ref.object_id {
            return Err(RetainedObjectError::ObjectIdMismatch);
        }
        if rederived.evidence_digest != expected_ref.evidence_digest {
            return Err(RetainedObjectError::EvidenceDigestMismatch);
        }
        Ok(bytes)
    }

    fn materialize<T: RetainedTypedValue>(
        &self,
        expected_ref: &ValueRef,
    ) -> Result<T, RetainedObjectError> {
        if expected_ref.schema_id != T::SCHEMA_ID {
            return Err(RetainedObjectError::WrongSchema {
                expected: T::SCHEMA_ID,
                actual: expected_ref.schema_id.clone(),
            });
        }
        T::decode(self.verified_bytes(expected_ref)?).ok_or(RetainedObjectError::DecodeFailure {
            schema_id: T::SCHEMA_ID,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BalanceSubject {
    account: String,
}

impl BalanceSubject {
    fn encoded(account: &str) -> Vec<u8> {
        format!("account:{account}").into_bytes()
    }
}

impl RetainedTypedValue for BalanceSubject {
    const SCHEMA_ID: &'static str = SUBJECT_SCHEMA;

    fn decode(bytes: &[u8]) -> Option<Self> {
        let text = std::str::from_utf8(bytes).ok()?;
        let account = text.strip_prefix("account:")?;
        (!account.is_empty() && account.len() <= 64).then(|| Self {
            account: account.to_owned(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BalanceResponse {
    amount_microunits: u64,
}

impl BalanceResponse {
    fn encoded(amount_microunits: u64) -> Vec<u8> {
        format!("amount:{amount_microunits}").into_bytes()
    }
}

impl RetainedTypedValue for BalanceResponse {
    const SCHEMA_ID: &'static str = RESPONSE_SCHEMA;

    fn decode(bytes: &[u8]) -> Option<Self> {
        let text = std::str::from_utf8(bytes).ok()?;
        let amount = text.strip_prefix("amount:")?;
        Some(Self {
            amount_microunits: amount.parse().ok()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TenantFactFrontier {
    store_scope_id: String,
    store_epoch: String,
    tenant_scope_id: String,
    fact_order: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ObjectField {
    Subject,
    Response,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ObjectPath {
    selected_ordinal: usize,
    field: ObjectField,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectedFact {
    fact_ref: String,
    fact_order: u64,
    subject_ref: ValueRef,
    response_ref: ValueRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FactSelectionResponse {
    authorization_ref: String,
    request_digest: Digest,
    frontier: TenantFactFrontier,
    selected: Vec<SelectedFact>,
}

impl FactSelectionResponse {
    fn prototype_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        append_sized(&mut bytes, self.authorization_ref.as_bytes());
        bytes.extend_from_slice(&self.request_digest);
        append_sized(&mut bytes, self.frontier.store_scope_id.as_bytes());
        append_sized(&mut bytes, self.frontier.store_epoch.as_bytes());
        append_sized(&mut bytes, self.frontier.tenant_scope_id.as_bytes());
        bytes.extend_from_slice(&self.frontier.fact_order.to_be_bytes());
        bytes.extend_from_slice(&(self.selected.len() as u64).to_be_bytes());
        for selected in &self.selected {
            append_sized(&mut bytes, selected.fact_ref.as_bytes());
            bytes.extend_from_slice(&selected.fact_order.to_be_bytes());
            selected.subject_ref.append_prototype_bytes(&mut bytes);
            selected.response_ref.append_prototype_bytes(&mut bytes);
        }
        bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObjectPathAuthority {
    authorization_ref: String,
    request_digest: Digest,
    frontier: TenantFactFrontier,
    paths: BTreeMap<ObjectPath, ValueRef>,
}

impl ObjectPathAuthority {
    fn for_response(response: &FactSelectionResponse) -> Self {
        let mut paths = BTreeMap::new();
        for (selected_ordinal, selected) in response.selected.iter().enumerate() {
            paths.insert(
                ObjectPath {
                    selected_ordinal,
                    field: ObjectField::Subject,
                },
                selected.subject_ref.clone(),
            );
            paths.insert(
                ObjectPath {
                    selected_ordinal,
                    field: ObjectField::Response,
                },
                selected.response_ref.clone(),
            );
        }
        Self {
            authorization_ref: response.authorization_ref.clone(),
            request_digest: response.request_digest,
            frontier: response.frontier.clone(),
            paths,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MaterializedFact {
    fact_ref: String,
    fact_order: u64,
    subject: BalanceSubject,
    response: BalanceResponse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResponseMaterializationError {
    AuthorizationBindingMismatch,
    RequestBindingMismatch,
    FrontierBindingMismatch,
    PathCountMismatch,
    FactBeyondFrontier,
    NonCanonicalSelectionOrder,
    UnreachableObjectPath(ObjectPath),
    PathReferenceMismatch(ObjectPath),
    Object {
        path: ObjectPath,
        source: RetainedObjectError,
    },
}

fn materialize_fact_response(
    response: &FactSelectionResponse,
    authority: &ObjectPathAuthority,
    objects: &RetainedObjectStore,
) -> Result<Vec<MaterializedFact>, ResponseMaterializationError> {
    if response.authorization_ref != authority.authorization_ref {
        return Err(ResponseMaterializationError::AuthorizationBindingMismatch);
    }
    if response.request_digest != authority.request_digest {
        return Err(ResponseMaterializationError::RequestBindingMismatch);
    }
    if response.frontier != authority.frontier {
        return Err(ResponseMaterializationError::FrontierBindingMismatch);
    }
    if authority.paths.len() != response.selected.len() * 2 {
        return Err(ResponseMaterializationError::PathCountMismatch);
    }

    let mut previous_key: Option<(u64, String)> = None;
    let materializer = RetainedObjectMaterializer::new(objects);
    let mut materialized = Vec::with_capacity(response.selected.len());
    for (selected_ordinal, selected) in response.selected.iter().enumerate() {
        if selected.fact_order > response.frontier.fact_order {
            return Err(ResponseMaterializationError::FactBeyondFrontier);
        }
        let selection_key = (selected.fact_order, selected.fact_ref.clone());
        if previous_key
            .as_ref()
            .is_some_and(|key| key >= &selection_key)
        {
            return Err(ResponseMaterializationError::NonCanonicalSelectionOrder);
        }
        previous_key = Some(selection_key);

        let subject_path = ObjectPath {
            selected_ordinal,
            field: ObjectField::Subject,
        };
        let response_path = ObjectPath {
            selected_ordinal,
            field: ObjectField::Response,
        };
        let authorized_subject = authority.paths.get(&subject_path).ok_or_else(|| {
            ResponseMaterializationError::UnreachableObjectPath(subject_path.clone())
        })?;
        let authorized_response = authority.paths.get(&response_path).ok_or_else(|| {
            ResponseMaterializationError::UnreachableObjectPath(response_path.clone())
        })?;
        if authorized_subject != &selected.subject_ref {
            return Err(ResponseMaterializationError::PathReferenceMismatch(
                subject_path,
            ));
        }
        if authorized_response != &selected.response_ref {
            return Err(ResponseMaterializationError::PathReferenceMismatch(
                response_path,
            ));
        }

        let subject = materializer
            .materialize::<BalanceSubject>(authorized_subject)
            .map_err(|source| ResponseMaterializationError::Object {
                path: ObjectPath {
                    selected_ordinal,
                    field: ObjectField::Subject,
                },
                source,
            })?;
        let typed_response = materializer
            .materialize::<BalanceResponse>(authorized_response)
            .map_err(|source| ResponseMaterializationError::Object {
                path: ObjectPath {
                    selected_ordinal,
                    field: ObjectField::Response,
                },
                source,
            })?;
        materialized.push(MaterializedFact {
            fact_ref: selected.fact_ref.clone(),
            fact_order: selected.fact_order,
            subject,
            response: typed_response,
        });
    }
    Ok(materialized)
}

fn retained_fact(
    objects: &mut RetainedObjectStore,
    fact_ref: &str,
    fact_order: u64,
    account: &str,
    amount_microunits: u64,
) -> SelectedFact {
    SelectedFact {
        fact_ref: fact_ref.to_owned(),
        fact_order,
        subject_ref: objects.retain(SUBJECT_SCHEMA, BalanceSubject::encoded(account)),
        response_ref: objects.retain(RESPONSE_SCHEMA, BalanceResponse::encoded(amount_microunits)),
    }
}

fn response_fixture() -> (
    RetainedObjectStore,
    FactSelectionResponse,
    ObjectPathAuthority,
) {
    let mut objects = RetainedObjectStore::default();
    let selected = vec![
        retained_fact(&mut objects, "fact:1:0", 1, "alice", 1_000),
        retained_fact(&mut objects, "fact:2:0", 2, "alice", 2_000),
    ];
    let response = FactSelectionResponse {
        authorization_ref: "authorization:7".to_owned(),
        request_digest: digest(b"mfm.test.request.v1", &[b"alice"]),
        frontier: TenantFactFrontier {
            store_scope_id: "store:prototype".to_owned(),
            store_epoch: "epoch:prototype".to_owned(),
            tenant_scope_id: "tenant:a".to_owned(),
            fact_order: 2,
        },
        selected,
    };
    let authority = ObjectPathAuthority::for_response(&response);
    (objects, response, authority)
}

#[test]
fn generic_object_path_materializer_preserves_response_bindings_and_explicit_empty() {
    let (objects, response, authority) = response_fixture();
    let materialized =
        materialize_fact_response(&response, &authority, &objects).expect("materialized response");

    assert_eq!(
        materialized,
        vec![
            MaterializedFact {
                fact_ref: "fact:1:0".to_owned(),
                fact_order: 1,
                subject: BalanceSubject {
                    account: "alice".to_owned()
                },
                response: BalanceResponse {
                    amount_microunits: 1_000
                },
            },
            MaterializedFact {
                fact_ref: "fact:2:0".to_owned(),
                fact_order: 2,
                subject: BalanceSubject {
                    account: "alice".to_owned()
                },
                response: BalanceResponse {
                    amount_microunits: 2_000
                },
            },
        ]
    );

    let empty_response = FactSelectionResponse {
        selected: Vec::new(),
        ..response
    };
    let empty_authority = ObjectPathAuthority::for_response(&empty_response);
    assert_eq!(
        materialize_fact_response(&empty_response, &empty_authority, &objects),
        Ok(Vec::new())
    );
}

#[test]
fn generic_object_path_materializer_rejects_context_order_and_reachability_substitution() {
    let (objects, response, authority) = response_fixture();

    let mut wrong_authorization = authority.clone();
    wrong_authorization.authorization_ref = "authorization:substituted".to_owned();
    assert_eq!(
        materialize_fact_response(&response, &wrong_authorization, &objects),
        Err(ResponseMaterializationError::AuthorizationBindingMismatch)
    );

    let mut wrong_request = authority.clone();
    wrong_request.request_digest = digest(b"mfm.test.request.v1", &[b"mallory"]);
    assert_eq!(
        materialize_fact_response(&response, &wrong_request, &objects),
        Err(ResponseMaterializationError::RequestBindingMismatch)
    );

    let mut wrong_frontier = authority.clone();
    wrong_frontier.frontier.fact_order = 1;
    assert_eq!(
        materialize_fact_response(&response, &wrong_frontier, &objects),
        Err(ResponseMaterializationError::FrontierBindingMismatch)
    );

    let mut reversed = response.clone();
    reversed.selected.reverse();
    let reversed_authority = ObjectPathAuthority::for_response(&reversed);
    assert_eq!(
        materialize_fact_response(&reversed, &reversed_authority, &objects),
        Err(ResponseMaterializationError::NonCanonicalSelectionOrder)
    );

    let mut unreachable = authority;
    let missing_path = ObjectPath {
        selected_ordinal: 0,
        field: ObjectField::Response,
    };
    let retained_ref = unreachable
        .paths
        .remove(&missing_path)
        .expect("reachable fixture path");
    unreachable.paths.insert(
        ObjectPath {
            selected_ordinal: 99,
            field: ObjectField::Response,
        },
        retained_ref,
    );
    assert_eq!(
        materialize_fact_response(&response, &unreachable, &objects),
        Err(ResponseMaterializationError::UnreachableObjectPath(
            missing_path
        ))
    );
}

#[test]
fn generic_object_path_materializer_rejects_missing_wrong_schema_tampered_and_wrong_evidence() {
    let (objects, response, _) = response_fixture();

    let mut missing_objects = objects.clone();
    missing_objects.remove(&response.selected[0].response_ref.object_id);
    let authority = ObjectPathAuthority::for_response(&response);
    assert!(matches!(
        materialize_fact_response(&response, &authority, &missing_objects),
        Err(ResponseMaterializationError::Object {
            source: RetainedObjectError::MissingObject,
            ..
        })
    ));

    let mut wrong_schema_response = response.clone();
    wrong_schema_response.selected[0].response_ref.schema_id = SUBJECT_SCHEMA.to_owned();
    let wrong_schema_authority = ObjectPathAuthority::for_response(&wrong_schema_response);
    assert!(matches!(
        materialize_fact_response(&wrong_schema_response, &wrong_schema_authority, &objects),
        Err(ResponseMaterializationError::Object {
            source: RetainedObjectError::WrongSchema { .. },
            ..
        })
    ));

    let mut tampered_objects = objects.clone();
    let tampered_ref = &response.selected[0].response_ref;
    tampered_objects.replace_bytes(tampered_ref.object_id, BalanceResponse::encoded(9_999));
    assert!(matches!(
        materialize_fact_response(&response, &authority, &tampered_objects),
        Err(ResponseMaterializationError::Object {
            source: RetainedObjectError::ContentDigestMismatch,
            ..
        })
    ));

    let mut wrong_evidence_response = response.clone();
    wrong_evidence_response.selected[0]
        .response_ref
        .evidence_digest[0] ^= 0x01;
    let wrong_evidence_authority = ObjectPathAuthority::for_response(&wrong_evidence_response);
    assert!(matches!(
        materialize_fact_response(
            &wrong_evidence_response,
            &wrong_evidence_authority,
            &objects
        ),
        Err(ResponseMaterializationError::Object {
            source: RetainedObjectError::EvidenceDigestMismatch,
            ..
        })
    ));
}

#[derive(Debug, Clone)]
struct StoredFact {
    fact_ref: String,
    descriptor: String,
    subject_ref: ValueRef,
    response_ref: ValueRef,
}

#[derive(Debug, Clone)]
struct FactPublication {
    fact_order: u64,
    source_run_id: String,
    facts: Vec<StoredFact>,
}

#[derive(Debug, Default)]
struct TenantFactJournal {
    publications: RefCell<BTreeMap<String, Vec<FactPublication>>>,
}

#[derive(Debug, Clone)]
struct FactQuery {
    tenant_scope_id: String,
    consumer_run_id: String,
    descriptor: String,
    account: String,
    minimum_amount_microunits: u64,
    limit: usize,
}

impl FactQuery {
    fn digest(&self) -> Digest {
        digest(
            b"mfm.test.fact-query.v1",
            &[
                self.tenant_scope_id.as_bytes(),
                self.consumer_run_id.as_bytes(),
                self.descriptor.as_bytes(),
                self.account.as_bytes(),
                &self.minimum_amount_microunits.to_be_bytes(),
                &(self.limit as u64).to_be_bytes(),
            ],
        )
    }
}

#[derive(Debug)]
struct ScanMeasurement {
    scanned_publications: usize,
    scanned_facts: usize,
    qualifying_facts: usize,
    hydrated_objects: usize,
    continuation_steps: usize,
    max_step_publications: usize,
    max_step_facts: usize,
    elapsed: Duration,
}

#[derive(Debug)]
struct AuditedScan {
    response: FactSelectionResponse,
    object_authority: ObjectPathAuthority,
    measurement: ScanMeasurement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScanError {
    ResultLimitExceeded,
    InvalidStepBudget,
    FrontierUnavailable,
    NonDensePublicationOrder,
    MeasurementOverflow,
    Object {
        fact_ref: String,
        source: RetainedObjectError,
    },
}

#[derive(Debug, Clone, Copy)]
struct ScanStepBudget {
    publications: usize,
    facts: usize,
}

impl ScanStepBudget {
    const DEFAULT: Self = Self {
        publications: MAX_SCAN_STEP_PUBLICATIONS,
        facts: MAX_SCAN_STEP_FACTS,
    };

    fn validate(self) -> Result<Self, ScanError> {
        if self.publications == 0 || self.facts == 0 {
            return Err(ScanError::InvalidStepBudget);
        }
        Ok(self)
    }
}

enum FactScanProgress<'a> {
    More(FactScanSession<'a>),
    Complete(Box<AuditedScan>),
}

struct FactScanSession<'a> {
    journal: &'a TenantFactJournal,
    authorization_ref: &'a str,
    frontier: &'a TenantFactFrontier,
    query: &'a FactQuery,
    prefix_len: usize,
    objects: &'a RetainedObjectStore,
    next_publication_index: usize,
    next_fact_index: usize,
    selected: Vec<SelectedFact>,
    scanned_publications: usize,
    scanned_facts: usize,
    qualifying_facts: usize,
    hydrated_objects: usize,
    continuation_steps: usize,
    max_step_publications: usize,
    max_step_facts: usize,
    started: Instant,
}

impl<'a> FactScanSession<'a> {
    fn new(
        journal: &'a TenantFactJournal,
        authorization_ref: &'a str,
        frontier: &'a TenantFactFrontier,
        query: &'a FactQuery,
        objects: &'a RetainedObjectStore,
    ) -> Result<Self, ScanError> {
        if query.limit > MAX_RESULT_LIMIT {
            return Err(ScanError::ResultLimitExceeded);
        }
        if query.tenant_scope_id != frontier.tenant_scope_id {
            return Err(ScanError::FrontierUnavailable);
        }

        let all_publications = journal.publications.borrow();
        let publications = all_publications
            .get(&query.tenant_scope_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let prefix_len = publications
            .iter()
            .position(|publication| publication.fact_order > frontier.fact_order)
            .unwrap_or(publications.len());
        if frontier.fact_order != prefix_len as u64 {
            return Err(ScanError::FrontierUnavailable);
        }
        for (index, publication) in publications[..prefix_len].iter().enumerate() {
            if publication.fact_order != index as u64 + 1 {
                return Err(ScanError::NonDensePublicationOrder);
            }
        }

        Ok(Self {
            journal,
            authorization_ref,
            frontier,
            query,
            prefix_len,
            objects,
            next_publication_index: 0,
            next_fact_index: 0,
            selected: Vec::with_capacity(query.limit),
            scanned_publications: 0,
            scanned_facts: 0,
            qualifying_facts: 0,
            hydrated_objects: 0,
            continuation_steps: 0,
            max_step_publications: 0,
            max_step_facts: 0,
            started: Instant::now(),
        })
    }

    fn step(mut self, budget: ScanStepBudget) -> Result<FactScanProgress<'a>, ScanError> {
        let budget = budget.validate()?;
        let materializer = RetainedObjectMaterializer::new(self.objects);
        let mut step_publications = 0_usize;
        let mut step_facts = 0_usize;
        let mut last_touched_publication = None;
        let all_publications = self.journal.publications.borrow();
        let publications = all_publications
            .get(&self.query.tenant_scope_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if publications.len() < self.prefix_len {
            return Err(ScanError::FrontierUnavailable);
        }

        while self.next_publication_index < self.prefix_len {
            let publication_index = self.next_publication_index;
            let publication = &publications[publication_index];
            if publication.fact_order != publication_index as u64 + 1 {
                return Err(ScanError::NonDensePublicationOrder);
            }

            if self.next_fact_index < publication.facts.len() && step_facts == budget.facts {
                break;
            }
            if last_touched_publication != Some(publication_index) {
                if step_publications == budget.publications {
                    break;
                }
                step_publications += 1;
                last_touched_publication = Some(publication_index);
                if self.next_fact_index == 0 {
                    self.scanned_publications = self
                        .scanned_publications
                        .checked_add(1)
                        .ok_or(ScanError::MeasurementOverflow)?;
                }
            }

            if self.next_fact_index == publication.facts.len() {
                self.next_publication_index += 1;
                self.next_fact_index = 0;
                continue;
            }

            let fact = &publication.facts[self.next_fact_index];
            self.next_fact_index += 1;
            self.scanned_facts = self
                .scanned_facts
                .checked_add(1)
                .ok_or(ScanError::MeasurementOverflow)?;
            step_facts += 1;
            if publication.source_run_id == self.query.consumer_run_id {
                continue;
            }

            let subject = materializer
                .materialize::<BalanceSubject>(&fact.subject_ref)
                .map_err(|source| ScanError::Object {
                    fact_ref: fact.fact_ref.clone(),
                    source,
                })?;
            self.hydrated_objects = self
                .hydrated_objects
                .checked_add(1)
                .ok_or(ScanError::MeasurementOverflow)?;
            let response = materializer
                .materialize::<BalanceResponse>(&fact.response_ref)
                .map_err(|source| ScanError::Object {
                    fact_ref: fact.fact_ref.clone(),
                    source,
                })?;
            self.hydrated_objects = self
                .hydrated_objects
                .checked_add(1)
                .ok_or(ScanError::MeasurementOverflow)?;

            if fact.descriptor == self.query.descriptor
                && subject.account == self.query.account
                && response.amount_microunits >= self.query.minimum_amount_microunits
            {
                self.qualifying_facts = self
                    .qualifying_facts
                    .checked_add(1)
                    .ok_or(ScanError::MeasurementOverflow)?;
                if self.selected.len() < self.query.limit {
                    self.selected.push(SelectedFact {
                        fact_ref: fact.fact_ref.clone(),
                        fact_order: publication.fact_order,
                        subject_ref: fact.subject_ref.clone(),
                        response_ref: fact.response_ref.clone(),
                    });
                }
            }
        }
        drop(all_publications);

        self.continuation_steps = self
            .continuation_steps
            .checked_add(1)
            .ok_or(ScanError::MeasurementOverflow)?;
        self.max_step_publications = self.max_step_publications.max(step_publications);
        self.max_step_facts = self.max_step_facts.max(step_facts);
        if self.next_publication_index == self.prefix_len && self.next_fact_index == 0 {
            Ok(FactScanProgress::Complete(Box::new(self.finish())))
        } else {
            Ok(FactScanProgress::More(self))
        }
    }

    fn finish(mut self) -> AuditedScan {
        self.selected.sort_by(|left, right| {
            (left.fact_order, &left.fact_ref).cmp(&(right.fact_order, &right.fact_ref))
        });

        let response = FactSelectionResponse {
            authorization_ref: self.authorization_ref.to_owned(),
            request_digest: self.query.digest(),
            frontier: self.frontier.clone(),
            selected: self.selected,
        };
        let object_authority = ObjectPathAuthority::for_response(&response);
        AuditedScan {
            response,
            object_authority,
            measurement: ScanMeasurement {
                scanned_publications: self.scanned_publications,
                scanned_facts: self.scanned_facts,
                qualifying_facts: self.qualifying_facts,
                hydrated_objects: self.hydrated_objects,
                continuation_steps: self.continuation_steps,
                max_step_publications: self.max_step_publications,
                max_step_facts: self.max_step_facts,
                elapsed: self.started.elapsed(),
            },
        }
    }

    fn run(self, budget: ScanStepBudget) -> Result<AuditedScan, ScanError> {
        let mut progress = self.step(budget)?;
        loop {
            match progress {
                FactScanProgress::More(session) => {
                    progress = session.step(budget)?;
                }
                FactScanProgress::Complete(scan) => return Ok(*scan),
            }
        }
    }
}

impl TenantFactJournal {
    fn append(&self, tenant_scope_id: &str, publication: FactPublication) {
        self.publications
            .borrow_mut()
            .entry(tenant_scope_id.to_owned())
            .or_default()
            .push(publication);
    }

    fn complete_scan(
        &self,
        authorization_ref: &str,
        frontier: &TenantFactFrontier,
        query: &FactQuery,
        objects: &RetainedObjectStore,
    ) -> Result<AuditedScan, ScanError> {
        self.complete_scan_with_budget(
            authorization_ref,
            frontier,
            query,
            objects,
            ScanStepBudget::DEFAULT,
        )
    }

    fn complete_scan_with_budget(
        &self,
        authorization_ref: &str,
        frontier: &TenantFactFrontier,
        query: &FactQuery,
        objects: &RetainedObjectStore,
        budget: ScanStepBudget,
    ) -> Result<AuditedScan, ScanError> {
        FactScanSession::new(self, authorization_ref, frontier, query, objects)?.run(budget)
    }

    fn verify_exact_scan(
        &self,
        candidate: &FactSelectionResponse,
        authorization_ref: &str,
        expected_frontier: &TenantFactFrontier,
        query: &FactQuery,
        objects: &RetainedObjectStore,
    ) -> Result<ScanMeasurement, ScanVerificationError> {
        let expected = self
            .complete_scan(authorization_ref, expected_frontier, query, objects)
            .map_err(ScanVerificationError::Scan)?;
        if candidate != &expected.response {
            return Err(ScanVerificationError::SelectedOrOmittedFact);
        }
        Ok(expected.measurement)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScanVerificationError {
    Scan(ScanError),
    SelectedOrOmittedFact,
}

struct RepresentativeScanFixture {
    journal: TenantFactJournal,
    objects: RetainedObjectStore,
    query: FactQuery,
    frontier: TenantFactFrontier,
    expected_selected_refs: Vec<String>,
}

fn representative_scan_fixture() -> RepresentativeScanFixture {
    const REPRESENTATIVE_PUBLICATIONS: usize = 2_048;
    const FRONTIER_ORDER: usize = 1_984;

    let journal = TenantFactJournal::default();
    let mut objects = RetainedObjectStore::default();
    let query = FactQuery {
        tenant_scope_id: "tenant:a".to_owned(),
        consumer_run_id: "run:consumer".to_owned(),
        descriptor: "balance".to_owned(),
        account: "target".to_owned(),
        minimum_amount_microunits: 1_000,
        limit: MAX_RESULT_LIMIT,
    };
    let mut expected_selected_refs = Vec::new();

    for index in 1..=REPRESENTATIVE_PUBLICATIONS {
        let fact_order = index as u64;
        let target_account = if index % 3 == 0 { "target" } else { "other" };
        let target_amount = index as u64 * 10;
        let first_ref = format!("fact:{fact_order}:0");
        let first = StoredFact {
            fact_ref: first_ref.clone(),
            descriptor: "balance".to_owned(),
            subject_ref: objects.retain(SUBJECT_SCHEMA, BalanceSubject::encoded(target_account)),
            response_ref: objects.retain(RESPONSE_SCHEMA, BalanceResponse::encoded(target_amount)),
        };
        let second = StoredFact {
            fact_ref: format!("fact:{fact_order}:1"),
            descriptor: "audit-noise".to_owned(),
            subject_ref: objects.retain(SUBJECT_SCHEMA, BalanceSubject::encoded("target")),
            response_ref: objects.retain(RESPONSE_SCHEMA, BalanceResponse::encoded(u64::MAX)),
        };
        if index <= FRONTIER_ORDER
            && target_account == query.account
            && target_amount >= query.minimum_amount_microunits
            && expected_selected_refs.len() < query.limit
        {
            expected_selected_refs.push(first_ref);
        }
        journal.append(
            &query.tenant_scope_id,
            FactPublication {
                fact_order,
                source_run_id: format!("run:producer:{index}"),
                facts: vec![first, second],
            },
        );
    }

    // This tenant is physically present but outside the selected tenant prefix.
    for index in 1..=32 {
        journal.append(
            "tenant:b",
            FactPublication {
                fact_order: index,
                source_run_id: format!("run:other-tenant:{index}"),
                facts: Vec::new(),
            },
        );
    }

    RepresentativeScanFixture {
        journal,
        objects,
        query,
        frontier: TenantFactFrontier {
            store_scope_id: "store:prototype".to_owned(),
            store_epoch: "epoch:prototype".to_owned(),
            tenant_scope_id: "tenant:a".to_owned(),
            fact_order: FRONTIER_ORDER as u64,
        },
        expected_selected_refs,
    }
}

#[test]
fn complete_scan_measures_large_same_tenant_prefix_and_rejects_omission() {
    let fixture = representative_scan_fixture();
    let scan = fixture
        .journal
        .complete_scan(
            "authorization:scan",
            &fixture.frontier,
            &fixture.query,
            &fixture.objects,
        )
        .expect("complete bounded scan");
    let selected_refs: Vec<_> = scan
        .response
        .selected
        .iter()
        .map(|fact| fact.fact_ref.clone())
        .collect();

    assert_eq!(selected_refs, fixture.expected_selected_refs);
    assert_eq!(scan.measurement.scanned_publications, 1_984);
    assert_eq!(scan.measurement.scanned_facts, 3_968);
    assert_eq!(scan.measurement.hydrated_objects, 7_936);
    assert_eq!(scan.measurement.qualifying_facts, 628);
    assert_eq!(scan.measurement.continuation_steps, 1);
    assert_eq!(scan.measurement.max_step_publications, 1_984);
    assert_eq!(scan.measurement.max_step_facts, 3_968);
    assert!(scan.response.selected.len() <= MAX_RESULT_LIMIT);
    assert_eq!(
        materialize_fact_response(&scan.response, &scan.object_authority, &fixture.objects)
            .expect("scan response materializes")
            .len(),
        MAX_RESULT_LIMIT
    );
    eprintln!(
        "complete same-tenant scan: publications={}, facts={}, steps={}, elapsed={:?}",
        scan.measurement.scanned_publications,
        scan.measurement.scanned_facts,
        scan.measurement.continuation_steps,
        scan.measurement.elapsed
    );

    let mut omitted = scan.response.clone();
    omitted.selected.remove(17);
    assert!(matches!(
        fixture.journal.verify_exact_scan(
            &omitted,
            "authorization:scan",
            &fixture.frontier,
            &fixture.query,
            &fixture.objects
        ),
        Err(ScanVerificationError::SelectedOrOmittedFact)
    ));

    let lowered_frontier = TenantFactFrontier {
        fact_order: fixture.frontier.fact_order - 1,
        ..fixture.frontier.clone()
    };
    let lowered = fixture
        .journal
        .complete_scan(
            "authorization:scan",
            &lowered_frontier,
            &fixture.query,
            &fixture.objects,
        )
        .expect("lower-frontier response");
    assert!(matches!(
        fixture.journal.verify_exact_scan(
            &lowered.response,
            "authorization:scan",
            &fixture.frontier,
            &fixture.query,
            &fixture.objects
        ),
        Err(ScanVerificationError::SelectedOrOmittedFact)
    ));
}

#[test]
fn fact_scan_budget_changes_only_step_count_and_preserves_exact_boundaries() {
    let fixture = representative_scan_fixture();
    let one_step = fixture
        .journal
        .complete_scan_with_budget(
            "authorization:scan",
            &fixture.frontier,
            &fixture.query,
            &fixture.objects,
            ScanStepBudget::DEFAULT,
        )
        .expect("one-step scan");
    let many_steps = fixture
        .journal
        .complete_scan_with_budget(
            "authorization:scan",
            &fixture.frontier,
            &fixture.query,
            &fixture.objects,
            ScanStepBudget {
                publications: 17,
                facts: 34,
            },
        )
        .expect("many-step scan");

    assert_eq!(
        one_step.response.prototype_bytes(),
        many_steps.response.prototype_bytes()
    );
    assert_eq!(one_step.object_authority, many_steps.object_authority);
    assert_eq!(one_step.measurement.continuation_steps, 1);
    assert!(many_steps.measurement.continuation_steps > 1);
    assert_eq!(
        (
            one_step.measurement.scanned_publications,
            one_step.measurement.scanned_facts,
            one_step.measurement.qualifying_facts,
            one_step.measurement.hydrated_objects,
        ),
        (
            many_steps.measurement.scanned_publications,
            many_steps.measurement.scanned_facts,
            many_steps.measurement.qualifying_facts,
            many_steps.measurement.hydrated_objects,
        )
    );
    assert_eq!(many_steps.measurement.scanned_publications, 1_984);
    assert_eq!(many_steps.measurement.scanned_facts, 3_968);
    assert_eq!(many_steps.measurement.max_step_publications, 17);
    assert_eq!(many_steps.measurement.max_step_facts, 34);
    let selected_refs: BTreeSet<_> = many_steps
        .response
        .selected
        .iter()
        .map(|fact| fact.fact_ref.as_str())
        .collect();
    assert_eq!(selected_refs.len(), many_steps.response.selected.len());
}

#[test]
fn fact_scan_continuation_exceeds_former_totals_and_restart_is_deterministic() {
    let fixture = representative_scan_fixture();
    let mut oversized_query = fixture.query.clone();
    oversized_query.limit = MAX_RESULT_LIMIT + 1;
    assert!(matches!(
        fixture.journal.complete_scan(
            "authorization:scan",
            &fixture.frontier,
            &oversized_query,
            &fixture.objects
        ),
        Err(ScanError::ResultLimitExceeded)
    ));

    let mut objects = RetainedObjectStore::default();
    let shared_fact = StoredFact {
        fact_ref: "fact:shared".to_owned(),
        descriptor: "balance".to_owned(),
        subject_ref: objects.retain(SUBJECT_SCHEMA, BalanceSubject::encoded("target")),
        response_ref: objects.retain(RESPONSE_SCHEMA, BalanceResponse::encoded(1_000)),
    };

    let publication_budget_journal = TenantFactJournal::default();
    for index in 1..=MAX_SCAN_STEP_PUBLICATIONS + 1 {
        publication_budget_journal.append(
            "tenant:a",
            FactPublication {
                fact_order: index as u64,
                source_run_id: fixture.query.consumer_run_id.clone(),
                facts: vec![shared_fact.clone()],
            },
        );
    }
    let publication_budget_frontier = TenantFactFrontier {
        fact_order: (MAX_SCAN_STEP_PUBLICATIONS + 1) as u64,
        ..fixture.frontier.clone()
    };
    let publication_continuation = FactScanSession::new(
        &publication_budget_journal,
        "authorization:publication-budget",
        &publication_budget_frontier,
        &fixture.query,
        &objects,
    )
    .expect("publication-budget session");
    let publication_continuation = match publication_continuation.step(ScanStepBudget::DEFAULT) {
        Ok(FactScanProgress::More(session)) => session,
        Ok(FactScanProgress::Complete(_)) => panic!("prefix must require continuation"),
        Err(error) => panic!("publication-budget step failed: {error:?}"),
    };
    assert_eq!(
        publication_continuation.next_publication_index,
        MAX_SCAN_STEP_PUBLICATIONS
    );
    assert_eq!(publication_continuation.next_fact_index, 0);
    let publication_complete = publication_continuation
        .run(ScanStepBudget::DEFAULT)
        .expect("publication total continues to completion");
    assert_eq!(
        publication_complete.measurement.scanned_publications,
        MAX_SCAN_STEP_PUBLICATIONS + 1
    );

    let larger_than_measurement_fixture = TenantFactJournal::default();
    let publication_count = 4_097;
    for index in 1..=publication_count {
        larger_than_measurement_fixture.append(
            "tenant:a",
            FactPublication {
                fact_order: index as u64,
                source_run_id: format!("run:{index}"),
                facts: vec![
                    shared_fact.clone(),
                    shared_fact.clone(),
                    shared_fact.clone(),
                ],
            },
        );
    }
    let large_frontier = TenantFactFrontier {
        fact_order: publication_count as u64,
        ..fixture.frontier.clone()
    };
    let mut no_match_query = fixture.query.clone();
    no_match_query.descriptor = "no-match".to_owned();
    let continued = FactScanSession::new(
        &larger_than_measurement_fixture,
        "authorization:scan",
        &large_frontier,
        &no_match_query,
        &objects,
    )
    .expect("large scan session");
    let dropped = match continued.step(ScanStepBudget::DEFAULT) {
        Ok(FactScanProgress::More(session)) => session,
        Ok(FactScanProgress::Complete(_)) => panic!("fact total must require continuation"),
        Err(error) => panic!("first bounded step failed: {error:?}"),
    };
    assert_eq!(dropped.next_fact_index, 2);
    drop(dropped);

    let restarted = larger_than_measurement_fixture
        .complete_scan(
            "authorization:scan",
            &large_frontier,
            &no_match_query,
            &objects,
        )
        .expect("restart deterministically rescans");
    let one_step = larger_than_measurement_fixture
        .complete_scan_with_budget(
            "authorization:scan",
            &large_frontier,
            &no_match_query,
            &objects,
            ScanStepBudget {
                publications: publication_count + 1,
                facts: publication_count * 3 + 1,
            },
        )
        .expect("same scan in one step");
    assert_eq!(
        restarted.response.prototype_bytes(),
        one_step.response.prototype_bytes()
    );
    assert_eq!(
        restarted.measurement.scanned_publications,
        publication_count
    );
    assert_eq!(restarted.measurement.scanned_facts, publication_count * 3);
    assert!(restarted.measurement.continuation_steps > 1);
    assert_eq!(one_step.measurement.continuation_steps, 1);
    assert!(restarted.measurement.max_step_publications <= MAX_SCAN_STEP_PUBLICATIONS);
    assert!(restarted.measurement.max_step_facts <= MAX_SCAN_STEP_FACTS);
}

#[test]
fn fact_scan_excludes_publications_appended_after_its_frontier() {
    let journal = TenantFactJournal::default();
    let mut objects = RetainedObjectStore::default();
    let query = FactQuery {
        tenant_scope_id: "tenant:a".to_owned(),
        consumer_run_id: "run:consumer".to_owned(),
        descriptor: "balance".to_owned(),
        account: "target".to_owned(),
        minimum_amount_microunits: 0,
        limit: MAX_RESULT_LIMIT,
    };
    for fact_order in 1..=6_u64 {
        let selected = retained_fact(
            &mut objects,
            &format!("fact:{fact_order}:0"),
            fact_order,
            "target",
            fact_order,
        );
        journal.append(
            &query.tenant_scope_id,
            FactPublication {
                fact_order,
                source_run_id: format!("run:producer:{fact_order}"),
                facts: vec![StoredFact {
                    fact_ref: selected.fact_ref,
                    descriptor: "balance".to_owned(),
                    subject_ref: selected.subject_ref,
                    response_ref: selected.response_ref,
                }],
            },
        );
    }
    let frontier = TenantFactFrontier {
        store_scope_id: "store:prototype".to_owned(),
        store_epoch: "epoch:prototype".to_owned(),
        tenant_scope_id: query.tenant_scope_id.clone(),
        fact_order: 6,
    };
    let post_frontier = retained_fact(&mut objects, "fact:7:0", 7, "target", 7);
    let session = FactScanSession::new(&journal, "authorization:scan", &frontier, &query, &objects)
        .expect("frontier session");
    let session = match session.step(ScanStepBudget {
        publications: 2,
        facts: 2,
    }) {
        Ok(FactScanProgress::More(session)) => session,
        Ok(FactScanProgress::Complete(_)) => panic!("scan must yield"),
        Err(error) => panic!("scan step failed: {error:?}"),
    };

    journal.append(
        &query.tenant_scope_id,
        FactPublication {
            fact_order: 7,
            source_run_id: "run:producer:7".to_owned(),
            facts: vec![StoredFact {
                fact_ref: post_frontier.fact_ref,
                descriptor: "balance".to_owned(),
                subject_ref: post_frontier.subject_ref,
                response_ref: post_frontier.response_ref,
            }],
        },
    );
    let complete = session
        .run(ScanStepBudget {
            publications: 2,
            facts: 2,
        })
        .expect("frontier scan completes");
    assert_eq!(complete.measurement.scanned_publications, 6);
    assert_eq!(complete.measurement.scanned_facts, 6);
    assert_eq!(complete.response.frontier.fact_order, 6);
    assert_eq!(complete.response.selected.len(), 6);
    assert!(complete
        .response
        .selected
        .iter()
        .all(|fact| fact.fact_order <= 6 && fact.fact_ref != "fact:7:0"));
}

#[derive(Debug, Clone)]
struct SourceProofNode {
    source_ref: String,
    dependencies: Vec<String>,
    object_refs: Vec<ValueRef>,
}

impl SourceProofNode {
    fn canonical_metadata_bytes(&self) -> usize {
        size_of::<u64>()
            + self.source_ref.len()
            + size_of::<u64>()
            + self
                .dependencies
                .iter()
                .map(|dependency| size_of::<u64>() + dependency.len())
                .sum::<usize>()
            + size_of::<u64>()
            + self
                .object_refs
                .iter()
                .map(ValueRef::canonical_metadata_bytes)
                .sum::<usize>()
    }
}

#[derive(Debug, Default)]
struct SourceProofGraph {
    nodes: BTreeMap<String, SourceProofNode>,
}

#[derive(Debug)]
struct ClosureMeasurement {
    source_count: usize,
    logical_object_references: usize,
    unique_object_count: usize,
    logical_object_bytes: usize,
    unique_object_bytes: usize,
    deduplicated_object_bytes_saved: usize,
    canonical_metadata_bytes: usize,
    total_closure_bytes: usize,
    verification_steps: usize,
    max_step_sources: usize,
    max_step_unique_objects: usize,
    max_step_bytes: usize,
    elapsed: Duration,
}

#[derive(Debug)]
struct PortableSourceClosure {
    source_refs: BTreeSet<String>,
    unique_objects: BTreeMap<Digest, ValueRef>,
    measurement: ClosureMeasurement,
}

impl PortableSourceClosure {
    fn prototype_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(self.source_refs.len() as u64).to_be_bytes());
        for source_ref in &self.source_refs {
            append_sized(&mut bytes, source_ref.as_bytes());
        }
        bytes.extend_from_slice(&(self.unique_objects.len() as u64).to_be_bytes());
        for value_ref in self.unique_objects.values() {
            value_ref.append_prototype_bytes(&mut bytes);
        }
        bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClosureError {
    InvalidStepBudget,
    MissingSource(String),
    CyclicSource(String),
    MeasurementOverflow,
    ConflictingObjectReference,
    Object(RetainedObjectError),
}

#[derive(Debug, Clone, Copy)]
struct ClosureStepBudget {
    sources: usize,
    unique_objects: usize,
    bytes: usize,
}

impl ClosureStepBudget {
    const DEFAULT: Self = Self {
        sources: MAX_CLOSURE_STEP_SOURCES,
        unique_objects: MAX_CLOSURE_STEP_OBJECT_REFS,
        bytes: MAX_CLOSURE_STEP_BYTES,
    };

    fn validate(self) -> Result<Self, ClosureError> {
        if self.sources == 0 || self.unique_objects == 0 || self.bytes == 0 {
            return Err(ClosureError::InvalidStepBudget);
        }
        Ok(self)
    }
}

enum ClosureProgress<'a> {
    More(ClosureSession<'a>),
    Complete(PortableSourceClosure),
}

enum ClosureVisit {
    Enter(String),
    SourceMetadata {
        source_ref: String,
        offset: usize,
        source_counted: bool,
    },
    Object {
        source_ref: String,
        object_index: usize,
        payload_offset: usize,
        logical_counted: bool,
        unique_counted: bool,
        object_verified: bool,
    },
}

struct ClosureSession<'a> {
    graph: &'a SourceProofGraph,
    objects: &'a RetainedObjectStore,
    stack: Vec<ClosureVisit>,
    active: BTreeSet<String>,
    source_refs: BTreeSet<String>,
    unique_objects: BTreeMap<Digest, ValueRef>,
    logical_object_references: usize,
    logical_object_bytes: usize,
    unique_object_bytes: usize,
    canonical_metadata_bytes: usize,
    verification_steps: usize,
    max_step_sources: usize,
    max_step_unique_objects: usize,
    max_step_bytes: usize,
    started: Instant,
}

impl<'a> ClosureSession<'a> {
    fn new(
        graph: &'a SourceProofGraph,
        roots: &[String],
        objects: &'a RetainedObjectStore,
    ) -> Self {
        let mut sorted_roots = roots.to_vec();
        sorted_roots.sort();
        sorted_roots.dedup();
        Self {
            graph,
            objects,
            stack: sorted_roots
                .into_iter()
                .rev()
                .map(ClosureVisit::Enter)
                .collect(),
            active: BTreeSet::new(),
            source_refs: BTreeSet::new(),
            unique_objects: BTreeMap::new(),
            logical_object_references: 0,
            logical_object_bytes: 0,
            unique_object_bytes: 0,
            canonical_metadata_bytes: 0,
            verification_steps: 0,
            max_step_sources: 0,
            max_step_unique_objects: 0,
            max_step_bytes: 0,
            started: Instant::now(),
        }
    }

    fn record_step(
        &mut self,
        step_sources: usize,
        step_unique_objects: usize,
        step_bytes: usize,
    ) -> Result<(), ClosureError> {
        self.verification_steps = self
            .verification_steps
            .checked_add(1)
            .ok_or(ClosureError::MeasurementOverflow)?;
        self.max_step_sources = self.max_step_sources.max(step_sources);
        self.max_step_unique_objects = self.max_step_unique_objects.max(step_unique_objects);
        self.max_step_bytes = self.max_step_bytes.max(step_bytes);
        Ok(())
    }

    fn more(
        mut self,
        step_sources: usize,
        step_unique_objects: usize,
        step_bytes: usize,
    ) -> Result<ClosureProgress<'a>, ClosureError> {
        self.record_step(step_sources, step_unique_objects, step_bytes)?;
        Ok(ClosureProgress::More(self))
    }

    fn step(mut self, budget: ClosureStepBudget) -> Result<ClosureProgress<'a>, ClosureError> {
        let budget = budget.validate()?;
        let materializer = RetainedObjectMaterializer::new(self.objects);
        let mut step_sources = 0_usize;
        let mut step_unique_objects = 0_usize;
        let mut step_bytes = 0_usize;

        loop {
            let Some(visit) = self.stack.pop() else {
                self.record_step(step_sources, step_unique_objects, step_bytes)?;
                return Ok(ClosureProgress::Complete(self.finish()?));
            };
            if step_bytes == budget.bytes {
                self.stack.push(visit);
                return self.more(step_sources, step_unique_objects, step_bytes);
            }

            match visit {
                ClosureVisit::Enter(source_ref) => {
                    if self.source_refs.contains(&source_ref) {
                        continue;
                    }
                    if !self.active.insert(source_ref.clone()) {
                        return Err(ClosureError::CyclicSource(source_ref));
                    }
                    let node = self
                        .graph
                        .nodes
                        .get(&source_ref)
                        .ok_or_else(|| ClosureError::MissingSource(source_ref.clone()))?;
                    if node.source_ref != source_ref {
                        return Err(ClosureError::MissingSource(source_ref));
                    }
                    self.canonical_metadata_bytes = self
                        .canonical_metadata_bytes
                        .checked_add(node.canonical_metadata_bytes())
                        .ok_or(ClosureError::MeasurementOverflow)?;
                    self.stack.push(ClosureVisit::SourceMetadata {
                        source_ref,
                        offset: 0,
                        source_counted: false,
                    });
                }
                ClosureVisit::SourceMetadata {
                    source_ref,
                    mut offset,
                    mut source_counted,
                } => {
                    if !source_counted {
                        if step_sources == budget.sources {
                            self.stack.push(ClosureVisit::SourceMetadata {
                                source_ref,
                                offset,
                                source_counted,
                            });
                            return self.more(step_sources, step_unique_objects, step_bytes);
                        }
                        step_sources += 1;
                        source_counted = true;
                    }
                    let node = self
                        .graph
                        .nodes
                        .get(&source_ref)
                        .ok_or_else(|| ClosureError::MissingSource(source_ref.clone()))?;
                    let metadata_bytes = node.canonical_metadata_bytes();
                    let consumed = (budget.bytes - step_bytes).min(metadata_bytes - offset);
                    offset += consumed;
                    step_bytes += consumed;
                    if offset < metadata_bytes {
                        self.stack.push(ClosureVisit::SourceMetadata {
                            source_ref,
                            offset,
                            source_counted,
                        });
                        return self.more(step_sources, step_unique_objects, step_bytes);
                    }

                    self.stack.push(ClosureVisit::Object {
                        source_ref: source_ref.clone(),
                        object_index: 0,
                        payload_offset: 0,
                        logical_counted: false,
                        unique_counted: false,
                        object_verified: false,
                    });
                    let mut dependencies = node.dependencies.clone();
                    dependencies.sort();
                    self.stack
                        .extend(dependencies.into_iter().rev().map(ClosureVisit::Enter));
                }
                ClosureVisit::Object {
                    source_ref,
                    object_index,
                    mut payload_offset,
                    mut logical_counted,
                    mut unique_counted,
                    mut object_verified,
                } => {
                    let node = self
                        .graph
                        .nodes
                        .get(&source_ref)
                        .ok_or_else(|| ClosureError::MissingSource(source_ref.clone()))?;
                    let Some(object_ref) = node.object_refs.get(object_index) else {
                        self.active.remove(&source_ref);
                        self.source_refs.insert(source_ref);
                        continue;
                    };

                    if !logical_counted {
                        self.logical_object_references = self
                            .logical_object_references
                            .checked_add(1)
                            .ok_or(ClosureError::MeasurementOverflow)?;
                        self.logical_object_bytes = self
                            .logical_object_bytes
                            .checked_add(object_ref.byte_length)
                            .ok_or(ClosureError::MeasurementOverflow)?;
                        logical_counted = true;
                    }
                    if let Some(existing) = self.unique_objects.get(&object_ref.object_id) {
                        if existing != object_ref {
                            return Err(ClosureError::ConflictingObjectReference);
                        }
                        self.stack.push(ClosureVisit::Object {
                            source_ref,
                            object_index: object_index + 1,
                            payload_offset: 0,
                            logical_counted: false,
                            unique_counted: false,
                            object_verified: false,
                        });
                        continue;
                    }
                    if !unique_counted {
                        if step_unique_objects == budget.unique_objects {
                            self.stack.push(ClosureVisit::Object {
                                source_ref,
                                object_index,
                                payload_offset,
                                logical_counted,
                                unique_counted,
                                object_verified,
                            });
                            return self.more(step_sources, step_unique_objects, step_bytes);
                        }
                        step_unique_objects += 1;
                        unique_counted = true;
                    }
                    if !object_verified {
                        materializer
                            .verified_bytes(object_ref)
                            .map_err(ClosureError::Object)?;
                        object_verified = true;
                    }

                    let consumed =
                        (budget.bytes - step_bytes).min(object_ref.byte_length - payload_offset);
                    payload_offset += consumed;
                    step_bytes += consumed;
                    if payload_offset < object_ref.byte_length {
                        self.stack.push(ClosureVisit::Object {
                            source_ref,
                            object_index,
                            payload_offset,
                            logical_counted,
                            unique_counted,
                            object_verified,
                        });
                        return self.more(step_sources, step_unique_objects, step_bytes);
                    }

                    self.unique_object_bytes = self
                        .unique_object_bytes
                        .checked_add(object_ref.byte_length)
                        .ok_or(ClosureError::MeasurementOverflow)?;
                    self.unique_objects
                        .insert(object_ref.object_id, object_ref.clone());
                    self.stack.push(ClosureVisit::Object {
                        source_ref,
                        object_index: object_index + 1,
                        payload_offset: 0,
                        logical_counted: false,
                        unique_counted: false,
                        object_verified: false,
                    });
                }
            }
        }
    }

    fn finish(self) -> Result<PortableSourceClosure, ClosureError> {
        let total_closure_bytes = self
            .canonical_metadata_bytes
            .checked_add(self.unique_object_bytes)
            .ok_or(ClosureError::MeasurementOverflow)?;
        let deduplicated_object_bytes_saved = self
            .logical_object_bytes
            .checked_sub(self.unique_object_bytes)
            .ok_or(ClosureError::MeasurementOverflow)?;
        Ok(PortableSourceClosure {
            measurement: ClosureMeasurement {
                source_count: self.source_refs.len(),
                logical_object_references: self.logical_object_references,
                unique_object_count: self.unique_objects.len(),
                logical_object_bytes: self.logical_object_bytes,
                unique_object_bytes: self.unique_object_bytes,
                deduplicated_object_bytes_saved,
                canonical_metadata_bytes: self.canonical_metadata_bytes,
                total_closure_bytes,
                verification_steps: self.verification_steps,
                max_step_sources: self.max_step_sources,
                max_step_unique_objects: self.max_step_unique_objects,
                max_step_bytes: self.max_step_bytes,
                elapsed: self.started.elapsed(),
            },
            source_refs: self.source_refs,
            unique_objects: self.unique_objects,
        })
    }

    fn run(self, budget: ClosureStepBudget) -> Result<PortableSourceClosure, ClosureError> {
        let mut progress = self.step(budget)?;
        loop {
            match progress {
                ClosureProgress::More(session) => progress = session.step(budget)?,
                ClosureProgress::Complete(closure) => return Ok(closure),
            }
        }
    }
}

impl SourceProofGraph {
    fn portable_closure(
        &self,
        roots: &[String],
        objects: &RetainedObjectStore,
    ) -> Result<PortableSourceClosure, ClosureError> {
        self.portable_closure_with_budget(roots, objects, ClosureStepBudget::DEFAULT)
    }

    fn portable_closure_with_budget(
        &self,
        roots: &[String],
        objects: &RetainedObjectStore,
        budget: ClosureStepBudget,
    ) -> Result<PortableSourceClosure, ClosureError> {
        ClosureSession::new(self, roots, objects).run(budget)
    }
}

struct RepresentativeClosureFixture {
    graph: SourceProofGraph,
    objects: RetainedObjectStore,
    root: String,
    expected_unique_object_bytes: usize,
    expected_logical_object_bytes: usize,
    expected_metadata_bytes: usize,
}

fn representative_closure_fixture() -> RepresentativeClosureFixture {
    const REPRESENTATIVE_SOURCES: usize = 128;

    let mut objects = RetainedObjectStore::default();
    let shared_spec = objects.retain(OPAQUE_SCHEMA, b"shared-certified-spec".to_vec());
    let shared_certificate = objects.retain(OPAQUE_SCHEMA, b"shared-certificate".to_vec());
    let mut graph = SourceProofGraph::default();
    let mut unique_ids = BTreeSet::from([shared_spec.object_id, shared_certificate.object_id]);
    let mut expected_logical_object_bytes = 0;
    let mut expected_metadata_bytes = 0;

    for index in 0..REPRESENTATIVE_SOURCES {
        let source_ref = format!("source:{index:03}");
        let transition = objects.retain(
            OPAQUE_SCHEMA,
            format!("transition-object:{index:03}").into_bytes(),
        );
        unique_ids.insert(transition.object_id);
        let object_refs = vec![shared_spec.clone(), shared_certificate.clone(), transition];
        expected_logical_object_bytes += object_refs
            .iter()
            .map(|object_ref| object_ref.byte_length)
            .sum::<usize>();
        let node = SourceProofNode {
            source_ref: source_ref.clone(),
            dependencies: if index > 0 {
                vec![format!("source:{:03}", index - 1)]
            } else {
                Vec::new()
            },
            object_refs,
        };
        expected_metadata_bytes += node.canonical_metadata_bytes();
        graph.nodes.insert(source_ref, node);
    }
    let expected_unique_object_bytes = unique_ids
        .iter()
        .map(|object_id| objects.objects[object_id].len())
        .sum();

    RepresentativeClosureFixture {
        graph,
        objects,
        root: format!("source:{:03}", REPRESENTATIVE_SOURCES - 1),
        expected_unique_object_bytes,
        expected_logical_object_bytes,
        expected_metadata_bytes,
    }
}

#[test]
fn portable_source_closure_is_byte_identical_across_consuming_step_budgets() {
    let fixture = representative_closure_fixture();
    let closure = fixture
        .graph
        .portable_closure(std::slice::from_ref(&fixture.root), &fixture.objects)
        .expect("bounded portable closure");
    let small_budget = ClosureStepBudget {
        sources: 8,
        unique_objects: 16,
        bytes: 2_048,
    };
    let many_steps = fixture
        .graph
        .portable_closure_with_budget(
            std::slice::from_ref(&fixture.root),
            &fixture.objects,
            small_budget,
        )
        .expect("many-step portable closure");

    assert_eq!(closure.source_refs.len(), 128);
    assert_eq!(closure.unique_objects.len(), 130);
    assert_eq!(closure.measurement.source_count, 128);
    assert_eq!(closure.measurement.logical_object_references, 384);
    assert_eq!(closure.measurement.unique_object_count, 130);
    assert_eq!(
        closure.measurement.unique_object_bytes,
        fixture.expected_unique_object_bytes
    );
    assert_eq!(
        closure.measurement.logical_object_bytes,
        fixture.expected_logical_object_bytes
    );
    assert_eq!(
        closure.measurement.deduplicated_object_bytes_saved,
        fixture.expected_logical_object_bytes - fixture.expected_unique_object_bytes
    );
    assert_eq!(
        closure.measurement.canonical_metadata_bytes,
        fixture.expected_metadata_bytes
    );
    assert_eq!(
        closure.measurement.total_closure_bytes,
        fixture.expected_metadata_bytes + fixture.expected_unique_object_bytes
    );
    assert_eq!(closure.measurement.verification_steps, 1);
    assert_eq!(closure.measurement.max_step_sources, 128);
    assert_eq!(closure.measurement.max_step_unique_objects, 130);
    assert_eq!(
        closure.measurement.max_step_bytes,
        closure.measurement.total_closure_bytes
    );
    assert_eq!(
        closure.prototype_bytes(),
        many_steps.prototype_bytes(),
        "step budget must not change closure bytes"
    );
    assert!(many_steps.measurement.verification_steps > 1);
    assert!(many_steps.measurement.max_step_sources <= small_budget.sources);
    assert!(many_steps.measurement.max_step_unique_objects <= small_budget.unique_objects);
    assert!(many_steps.measurement.max_step_bytes <= small_budget.bytes);
    assert_eq!(
        (
            closure.measurement.source_count,
            closure.measurement.logical_object_references,
            closure.measurement.unique_object_count,
            closure.measurement.logical_object_bytes,
            closure.measurement.unique_object_bytes,
            closure.measurement.canonical_metadata_bytes,
            closure.measurement.total_closure_bytes,
        ),
        (
            many_steps.measurement.source_count,
            many_steps.measurement.logical_object_references,
            many_steps.measurement.unique_object_count,
            many_steps.measurement.logical_object_bytes,
            many_steps.measurement.unique_object_bytes,
            many_steps.measurement.canonical_metadata_bytes,
            many_steps.measurement.total_closure_bytes,
        )
    );

    let dropped = match ClosureSession::new(
        &fixture.graph,
        std::slice::from_ref(&fixture.root),
        &fixture.objects,
    )
    .step(small_budget)
    {
        Ok(ClosureProgress::More(session)) => session,
        Ok(ClosureProgress::Complete(_)) => panic!("small budget must yield"),
        Err(error) => panic!("closure step failed: {error:?}"),
    };
    drop(dropped);
    let restarted = fixture
        .graph
        .portable_closure(std::slice::from_ref(&fixture.root), &fixture.objects)
        .expect("dropped closure session restarts deterministically");
    assert_eq!(closure.prototype_bytes(), restarted.prototype_bytes());
    eprintln!(
        "portable closure: sources={}, logical objects={}, unique objects={}, bytes={}, steps={}, elapsed={:?}",
        closure.measurement.source_count,
        closure.measurement.logical_object_references,
        closure.measurement.unique_object_count,
        closure.measurement.total_closure_bytes,
        closure.measurement.verification_steps,
        closure.measurement.elapsed
    );
}

#[test]
fn portable_source_closure_rejects_invalid_graphs_and_continues_past_step_budgets() {
    let fixture = representative_closure_fixture();
    let mut missing = fixture.graph;
    missing
        .nodes
        .get_mut(&fixture.root)
        .expect("root node")
        .dependencies = vec!["source:missing".to_owned()];
    assert!(matches!(
        missing.portable_closure(std::slice::from_ref(&fixture.root), &fixture.objects),
        Err(ClosureError::MissingSource(source_ref)) if source_ref == "source:missing"
    ));

    let mut missing_object_store = RetainedObjectStore::default();
    let missing_object_ref = missing_object_store.retain(OPAQUE_SCHEMA, b"missing-object".to_vec());
    missing_object_store.remove(&missing_object_ref.object_id);
    let missing_object_root = "missing-object-root".to_owned();
    let missing_object_graph = SourceProofGraph {
        nodes: BTreeMap::from([(
            missing_object_root.clone(),
            SourceProofNode {
                source_ref: missing_object_root.clone(),
                dependencies: Vec::new(),
                object_refs: vec![missing_object_ref],
            },
        )]),
    };
    assert!(matches!(
        missing_object_graph.portable_closure(
            std::slice::from_ref(&missing_object_root),
            &missing_object_store
        ),
        Err(ClosureError::Object(RetainedObjectError::MissingObject))
    ));

    let cyclic_root = "cyclic:a".to_owned();
    let cyclic_graph = SourceProofGraph {
        nodes: BTreeMap::from([
            (
                cyclic_root.clone(),
                SourceProofNode {
                    source_ref: cyclic_root.clone(),
                    dependencies: vec!["cyclic:b".to_owned()],
                    object_refs: Vec::new(),
                },
            ),
            (
                "cyclic:b".to_owned(),
                SourceProofNode {
                    source_ref: "cyclic:b".to_owned(),
                    dependencies: vec![cyclic_root.clone()],
                    object_refs: Vec::new(),
                },
            ),
        ]),
    };
    assert!(matches!(
        cyclic_graph.portable_closure(std::slice::from_ref(&cyclic_root), &fixture.objects),
        Err(ClosureError::CyclicSource(source_ref)) if source_ref == cyclic_root
    ));

    let mut multi_step_source_graph = SourceProofGraph::default();
    for index in 0..=MAX_CLOSURE_STEP_SOURCES {
        let source_ref = format!("bounded-source:{index}");
        multi_step_source_graph.nodes.insert(
            source_ref.clone(),
            SourceProofNode {
                source_ref,
                dependencies: if index > 0 {
                    vec![format!("bounded-source:{}", index - 1)]
                } else {
                    Vec::new()
                },
                object_refs: Vec::new(),
            },
        );
    }
    let source_bound_root = format!("bounded-source:{MAX_CLOSURE_STEP_SOURCES}");
    let multi_step_sources = multi_step_source_graph
        .portable_closure(std::slice::from_ref(&source_bound_root), &fixture.objects)
        .expect("source total continues across steps");
    let restarted_sources = multi_step_source_graph
        .portable_closure(std::slice::from_ref(&source_bound_root), &fixture.objects)
        .expect("closure verification restarts from immutable inputs");
    assert_eq!(
        multi_step_sources.source_refs,
        restarted_sources.source_refs
    );
    assert_eq!(
        multi_step_sources.measurement.source_count,
        MAX_CLOSURE_STEP_SOURCES + 1
    );
    assert_eq!(multi_step_sources.measurement.verification_steps, 2);
    assert!(multi_step_sources.measurement.max_step_sources <= MAX_CLOSURE_STEP_SOURCES);

    let mut multi_step_object_store = RetainedObjectStore::default();
    let object_refs = (0..=MAX_CLOSURE_STEP_OBJECT_REFS)
        .map(|index| {
            multi_step_object_store.retain(
                OPAQUE_SCHEMA,
                format!("bounded-object:{index}").into_bytes(),
            )
        })
        .collect();
    let object_bound_root = "multi-step-object-root".to_owned();
    let multi_step_object_graph = SourceProofGraph {
        nodes: BTreeMap::from([(
            object_bound_root.clone(),
            SourceProofNode {
                source_ref: object_bound_root.clone(),
                dependencies: Vec::new(),
                object_refs,
            },
        )]),
    };
    let multi_step_objects = multi_step_object_graph
        .portable_closure(
            std::slice::from_ref(&object_bound_root),
            &multi_step_object_store,
        )
        .expect("object total continues across steps");
    assert_eq!(
        multi_step_objects.measurement.unique_object_count,
        MAX_CLOSURE_STEP_OBJECT_REFS + 1
    );
    assert!(multi_step_objects.measurement.verification_steps >= 2);
    assert!(multi_step_objects.measurement.max_step_unique_objects <= MAX_CLOSURE_STEP_OBJECT_REFS);

    let mut multi_step_byte_store = RetainedObjectStore::default();
    let byte_refs = (0..300_u32)
        .map(|index| {
            let mut bytes = index.to_be_bytes().to_vec();
            bytes.resize(1_024, 0x5a);
            multi_step_byte_store.retain(OPAQUE_SCHEMA, bytes)
        })
        .collect();
    let byte_bound_root = "multi-step-byte-root".to_owned();
    let multi_step_byte_graph = SourceProofGraph {
        nodes: BTreeMap::from([(
            byte_bound_root.clone(),
            SourceProofNode {
                source_ref: byte_bound_root.clone(),
                dependencies: Vec::new(),
                object_refs: byte_refs,
            },
        )]),
    };
    let multi_step_bytes = multi_step_byte_graph
        .portable_closure(
            std::slice::from_ref(&byte_bound_root),
            &multi_step_byte_store,
        )
        .expect("byte total continues across steps");
    assert!(multi_step_bytes.measurement.total_closure_bytes > MAX_CLOSURE_STEP_BYTES);
    assert!(multi_step_bytes.measurement.verification_steps >= 2);
    assert!(multi_step_bytes.measurement.max_step_bytes <= MAX_CLOSURE_STEP_BYTES);
}

#[test]
fn continuation_sessions_are_private_consuming_and_not_serialized() {
    let source = include_str!("fact_response_materializer_prototype.rs");
    let fact_session = ["struct FactScan", "Session"].concat();
    let fact_progress = ["enum FactScan", "Progress"].concat();
    let closure_session = ["struct Closure", "Session"].concat();
    let closure_progress = ["enum Closure", "Progress"].concat();

    for type_name in [
        fact_session.as_str(),
        fact_progress.as_str(),
        closure_session.as_str(),
        closure_progress.as_str(),
    ] {
        assert!(source.contains(type_name));
        assert!(!source.contains(&["pub ", type_name].concat()));
        assert!(!source.contains(&["pub(crate) ", type_name].concat()));
    }

    let scan_cursor_type = ["FactScan", "Cursor"].concat();
    let scan_cursor_domain = ["fact-scan-", "cursor"].concat();
    assert!(!source.contains(&scan_cursor_type));
    assert!(!source.contains(&scan_cursor_domain));
    assert!(source.contains(&["fn step(mut self, budget: Scan", "StepBudget)"].concat()));
    assert!(source.contains(&["fn step(mut self, budget: Closure", "StepBudget)"].concat()));

    for type_name in [fact_session.as_str(), closure_session.as_str()] {
        let type_offset = source.find(type_name).expect("private session declaration");
        let declaration_prefix = &source[type_offset.saturating_sub(160)..type_offset];
        assert!(!declaration_prefix.contains("Clone"));
        assert!(!declaration_prefix.contains(&["Seria", "lize"].concat()));
        assert!(!source.contains(&["impl ", "Seria", "lize for ", type_name].concat()));
    }
}
