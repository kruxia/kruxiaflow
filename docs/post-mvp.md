# StreamFlow Post-MVP Roadmap

**Version**: 0.2
**Last Updated**: 2025-10-29
**Status**: Planning

---

## Overview

This document captures features, enhancements, and optimizations that are intentionally deferred beyond the MVP release. These items are organized into epics and user stories to provide a clear roadmap for StreamFlow's evolution from MVP to production-scale deployment.

The MVP focuses on:
- ✅ Single binary deployment
- ✅ PostgreSQL-only infrastructure (Queue, Events, Storage, Auth)
- ✅ Event-driven orchestration with polling
- ✅ Basic workflow execution (sequential, parallel, conditional)
- ✅ >1,000 workflows/sec target throughput
- ✅ Single-tenant deployments

Post-MVP expands to:
- 🔮 External service integrations (Auth0/Okta, Kafka, S3, Redis)
- 🔮 Multi-tenancy and advanced authorization
- 🔮 Performance optimizations (compiled workflows, caching, partitioning)
- 🔮 Developer experience enhancements (SDKs, UI, advanced expressions)
- 🔮 Enterprise features (monitoring, high availability, disaster recovery)

---

## Table of Contents

1. [Epic 1: External Service Integrations](#epic-1-external-service-integrations)
2. [Epic 2: Performance Optimization](#epic-2-performance-optimization)
3. [Epic 3: Multi-Tenancy & Authorization](#epic-3-multi-tenancy--authorization)
4. [Epic 4: Developer Experience](#epic-4-developer-experience)
5. [Epic 5: Enterprise Operations](#epic-5-enterprise-operations)
6. [Epic 6: Advanced Workflow Features](#epic-6-advanced-workflow-features)
7. [Epic 7: Scalability Enhancements](#epic-7-scalability-enhancements)

---

## Epic 1: External Service Integrations

**Goal**: Enable StreamFlow to integrate with external managed services for improved scalability, reliability, and operational simplicity in production environments.

### Story 1.1: Refresh Token Rotation with Grace Period

**Priority**: P1 (High - Security enhancement for MVP auth)

**As** a platform engineer
**I want** refresh token rotation with grace period handling
**So that** we can detect token theft while handling legitimate network failures

**Current Status**: MVP implements **strict rotation** (immediate revocation) per RFC 6749. This is secure but doesn't handle network failures gracefully.

**Scope**:
- Add `replaced_by` column to `oauth_refresh_tokens` table
- Implement grace period for token rotation (30 seconds default)
- Track token replacement chains (old → new)
- Detect reuse of replaced tokens:
  - Within grace period → Allow (network retry)
  - Outside grace period → Revoke both tokens (detected breach)
- Automatic cleanup of stale replaced tokens
- Metrics on token rotation and breach detection
- Configuration for grace period duration

**Architecture Reference**: `oauth/src/postgres.rs:210` (refresh_token method)

**Migration**:
```sql
ALTER TABLE oauth_refresh_tokens ADD COLUMN replaced_by UUID REFERENCES oauth_refresh_tokens(id);
CREATE INDEX idx_oauth_refresh_tokens_replaced ON oauth_refresh_tokens(replaced_by) WHERE replaced_by IS NOT NULL;
```

**Security Model**:
```
Token Lifecycle:
1. Token A issued at T0
2. Token A used at T1 → Replaced by Token B
3. Token A marked with replaced_by = Token B (not revoked yet)
4. If Token A used again:
   - T1 + 30s (within grace) → Return Token B (network retry)
   - T1 + 60s (outside grace) → Revoke both A and B (breach detected)
```

**Benefits**:
- **Security**: Detect token theft (reuse of replaced token)
- **Reliability**: Handle network failures gracefully
- **Industry standard**: Same approach as Auth0, Okta, Google
- **Observability**: Metrics on breaches and retries

**Trade-offs**:
- Increased complexity (token relationship tracking)
- Schema change required
- Grace period is a security vs reliability balance

**Post-MVP Enhancement**: This moves MVP from strict rotation to rotation with grace period, matching production OAuth providers.

---

### Story 1.2: External Identity Provider Integration

**Priority**: P1 (High - Common enterprise requirement)

**As** an enterprise platform engineer
**I want** to integrate StreamFlow with our existing identity provider (Auth0/Okta)
**So that** users can authenticate using SSO and we can manage access centrally

**Scope**:
- Implement Auth0 provider for `AuthenticationService` interface
- Implement Okta provider for `AuthenticationService` interface
- Support OIDC/OAuth2 token validation
- Support multiple token issuers (multi-provider setup)
- Configuration via environment variables (seamless switch from MVP)
- JWT signature verification with external JWKS endpoints
- Token refresh flows
- User/client management via external provider APIs

**Architecture Reference**: `docs/architecture.md` (AuthenticationService interface)

**Benefits**:
- Central identity management across organization
- SSO for users
- Advanced security features (MFA, conditional access)
- Reduced operational overhead (no user database management)
- Compliance with enterprise security policies

**Configuration Example**:
```bash
# Switch from MVP custom auth to Auth0
STREAMFLOW_OAUTH_PROVIDER=auth0
STREAMFLOW_OAUTH_DOMAIN=company.auth0.com
STREAMFLOW_OAUTH_AUDIENCE=streamflow-api
STREAMFLOW_OAUTH_CLIENT_ID=streamflow_server
STREAMFLOW_OAUTH_CLIENT_SECRET=...
```

---

### Story 1.3: Kafka/Redpanda Event Streaming

**Priority**: P1 (High - Key scalability path)

**As** a platform engineer scaling StreamFlow
**I want** to use Kafka/Redpanda for event streaming instead of PostgreSQL polling
**So that** I can handle >100,000 events/sec with lower latency

**Scope**:
- Implement Kafka provider for `EventSource` interface
- Support Redpanda (Kafka-compatible, simpler operations)
- Support WarpStream (S3-backed serverless Kafka)
- Consumer group management for multiple orchestrators
- Partition assignment and rebalancing
- Offset management (commit strategies)
- Error handling and retry policies
- Schema registry integration (optional)
- Monitoring via Kafka metrics

**Architecture Reference**: `docs/architecture.md` (EventSource interface)

**Performance Targets**:
- >100,000 events/sec throughput
- <5ms P95 event delivery latency
- Horizontal scaling with consumer groups

**Benefits**:
- 10-100x higher throughput than PostgreSQL polling
- Lower latency (<5ms vs ~10ms)
- Better separation of concerns (event streaming vs database)
- Multi-consumer patterns (analytics, audit, monitoring)
- Long event retention (days to weeks)

**Configuration Example**:
```bash
STREAMFLOW_EVENT_SOURCE=kafka
STREAMFLOW_KAFKA_BROKERS=kafka1:9092,kafka2:9092,kafka3:9092
STREAMFLOW_KAFKA_TOPIC=streamflow-events
STREAMFLOW_KAFKA_CONSUMER_GROUP=streamflow-orchestrators
```

---

### Story 1.4: NATS JetStream Event Streaming

**Priority**: P2 (Medium - Alternative for lower scale, simpler ops)

**As** a platform engineer deploying StreamFlow at moderate scale
**I want** to use NATS JetStream for event streaming
**So that** I get sub-millisecond latency with simpler operations than Kafka

**Scope**:
- Implement NATS provider for `EventSource` interface
- JetStream stream and consumer management
- Message acknowledgment and redelivery
- Horizontal scaling with queue groups
- Stream retention policies
- Monitoring via NATS server metrics

**Architecture Reference**: `docs/architecture.md` (EventSource interface)

**Performance Targets**:
- ~50,000 events/sec throughput
- <1ms P95 event delivery latency
- Simpler operations than Kafka

**Benefits**:
- Very low latency (<1ms)
- Simpler deployment and operations than Kafka
- Lower resource overhead
- Good fit for edge deployments with moderate scale

---

### Story 1.5: PostgreSQL Logical Replication Event Streaming

**Priority**: P2 (Medium - Stay with PostgreSQL while improving latency)

**As** a platform engineer
**I want** to use PostgreSQL logical replication for event streaming
**So that** I can improve latency while staying with PostgreSQL infrastructure

**Scope**:
- Implement logical replication provider for `EventSource` interface
- Replication slot management (create, monitor, cleanup)
- Change data capture (CDC) from `workflow_events` table
- Handle replication lag and slot growth
- Multiple consumer support via multiple slots
- Monitoring and alerting for replication health

**Architecture Reference**: `docs/architecture.md` (EventSource interface)

**Performance Targets**:
- ~10,000 events/sec throughput
- <10ms P95 event delivery latency (better than polling)
- Guaranteed delivery (replication slot tracks position)

**Benefits**:
- Improved latency over polling (<10ms vs ~10-5000ms)
- Stay with PostgreSQL (no new infrastructure)
- Guaranteed delivery (replication slots)
- Push model instead of poll

**Trade-offs**:
- More complex than polling (replication slot management)
- Replication slots require careful monitoring

---

### Story 1.6: AWS SQS Activity Queue

**Priority**: P2 (Medium - Cloud-native queue for AWS deployments)

**As** a platform engineer deploying StreamFlow on AWS
**I want** to use AWS SQS for the activity queue
**So that** I get a fully managed, scalable queue service

**Scope**:
- Implement SQS provider for `ActivityQueue` interface
- Message visibility timeout management
- Dead letter queue configuration
- Long polling support
- Message batching for performance
- FIFO queue support (optional, for strict ordering)
- Monitoring via CloudWatch metrics

**Architecture Reference**: `docs/architecture.md` (ActivityQueue interface)

**Benefits**:
- Fully managed (no queue maintenance)
- Automatic scaling
- Pay-per-use pricing
- High availability built-in
- Integration with AWS IAM

**Trade-offs**:
- Higher latency than PostgreSQL (~10-50ms vs <5ms)
- External dependency (vendor lock-in)
- Additional cost

---

### Story 1.7: RabbitMQ Activity Queue

**Priority**: P2 (Medium - High-throughput queue)

**As** a platform engineer requiring very high throughput
**I want** to use RabbitMQ for the activity queue
**So that** I can handle >50,000 activities/sec

**Scope**:
- Implement RabbitMQ provider for `ActivityQueue` interface
- Queue and exchange management
- Message acknowledgment and redelivery
- Prefetch and QoS settings
- Dead letter exchange configuration
- Cluster support (high availability)
- Monitoring via RabbitMQ metrics

**Benefits**:
- Very high throughput (>50,000 activities/sec)
- Low latency (<1ms)
- Mature, battle-tested
- Rich routing features

---

### Story 1.8: Redis Activity Queue

**Priority**: P3 (Lower - Niche use case)

**As** a platform engineer optimizing for latency
**I want** to use Redis for the activity queue
**So that** I get sub-millisecond queue operations

**Scope**:
- Implement Redis provider for `ActivityQueue` interface
- Redis Streams or List-based queue
- Consumer groups (Redis Streams)
- Message acknowledgment
- Persistence configuration (RDB/AOF)
- Redis Cluster support

**Benefits**:
- Very low latency (<1ms)
- Very high throughput (>100,000 activities/sec)
- In-memory performance

**Trade-offs**:
- Requires careful persistence setup
- More memory intensive

---

### Story 1.9: S3-Compatible Storage for Artifacts

**Priority**: P1 (High - Common for large files)

**As** a workflow developer handling large files
**I want** to store workflow artifacts in S3
**So that** I can handle files >2GB and leverage S3's durability and CDN integration

**Scope**:
- Implement S3 provider for `WorkflowStorage` interface
- Support AWS S3, MinIO, Cloudflare R2, etc.
- Multipart upload for large files (>5MB)
- Presigned URLs for direct client uploads
- Lifecycle policies for automatic cleanup
- Versioning support (optional)
- Server-side encryption configuration

**Architecture Reference**: `docs/architecture.md` (WorkflowStorage interface)

**Benefits**:
- No size limits (PostgreSQL Large Objects limited to ~2GB)
- Highly durable (99.999999999% durability)
- CDN integration (fast downloads)
- Scalable storage
- Cost-effective for large files

**Configuration Example**:
```bash
STREAMFLOW_STORAGE_PROVIDER=s3
STREAMFLOW_STORAGE_S3_BUCKET=streamflow-artifacts
STREAMFLOW_STORAGE_S3_ENDPOINT=https://s3.amazonaws.com
STREAMFLOW_STORAGE_S3_REGION=us-east-1
```

---

### Story 1.10: Filesystem Storage for Artifacts

**Priority**: P2 (Medium - Edge deployments)

**As** an edge deployment engineer
**I want** to store workflow artifacts on the local filesystem
**So that** I can run StreamFlow in air-gapped environments

**Scope**:
- Implement filesystem provider for `WorkflowStorage` interface
- Directory structure management (per-workflow directories)
- File permissions and ownership
- Disk space monitoring and alerts
- Cleanup of expired artifacts
- Support for NFS/network filesystems

**Benefits**:
- Simple deployment (no cloud dependencies)
- Works in air-gapped environments
- No network latency for storage access
- Predictable costs

**Trade-offs**:
- Single node storage (no replication)
- Manual backup/restore required
- Disk space management needed

---

### Story 1.11: Per-Workflow Storage Configuration

**Priority**: P2 (Medium - Enterprise requirement, complements Story 1.9)

**As** an enterprise workflow developer
**I want** to specify external object storage per workflow
**So that** workflow artifacts stay in my organization's storage for compliance and cost control

**Background**:
Currently, all workflows use a single system-wide storage backend configured via environment variables. This requires copying data from user-owned buckets into StreamFlow's managed storage, which:
- Increases storage costs (duplicate data)
- Creates compliance issues (sensitive data must stay in specific buckets)
- Adds latency (cross-bucket transfers)
- Complicates data sovereignty requirements

**Scope**:
- Workflow-level storage configuration in YAML workflow definitions
- Storage factory pattern to create per-workflow storage instances
- Support for workflow-specific credentials via Secrets Management
- Storage instance caching (reuse across workflow executions)
- Fallback behavior when user storage unavailable
- Lifecycle management (cleanup cached storage instances)
- Security validation (workflow can only access its designated storage)
- Documentation and examples for external storage setup

**Architecture Reference**: `docs/architecture.md` (WorkflowStorage interface)

**Dependencies**:
- US-5.4 (Object Storage and File Management) - base infrastructure
- US-11.6 (Secrets Management) - credential storage for user buckets
- Story 1.9 (S3-Compatible Storage) - S3 provider implementation

**Workflow Configuration Example**:
```yaml
workflow: etl_pipeline
version: "1.0"

# Workflow-level storage configuration
storage:
  provider: s3
  bucket: my-company-data-lake
  region: us-west-2
  endpoint: https://s3.us-west-2.amazonaws.com
  credentials_secret: etl_pipeline_s3_creds  # Reference to secret in Secrets Management
  path_prefix: "workflows/etl"  # Optional: scope to specific path
  retention_days: 30  # Optional: lifecycle policy

activities:
  - key: extract_data
    worker: data
    name: download_file
    parameters:
      source_url: "{{ARG.source_url}}"
      artifact_key: "raw_data.csv"
    outputs:
      - artifact_ref  # Stored in my-company-data-lake/workflows/etl/...
```

**Alternative: Reference existing user data without copy**:
```yaml
workflow: process_user_data
version: "1.0"

storage:
  provider: s3
  bucket: user-uploads-bucket  # User's existing bucket
  region: eu-west-1
  credentials_secret: user_bucket_readonly_creds
  mode: reference_only  # Don't copy, just reference existing files

activities:
  - key: analyze_file
    worker: analytics
    name: process_csv
    parameters:
      # Reference file already in user's bucket (no copy needed)
      input_file: "{{ARG.existing_file_key}}"
```

**Implementation Details**:

**Storage Factory** (Rust):
```rust
pub struct WorkflowStorageFactory {
    default_storage: Arc<dyn WorkflowStorage>,
    secrets_manager: Arc<dyn SecretsManager>,
    storage_cache: RwLock<HashMap<String, Arc<dyn WorkflowStorage>>>,
}

impl WorkflowStorageFactory {
    pub async fn get_storage(
        &self,
        workflow_def: &WorkflowDefinition,
    ) -> Result<Arc<dyn WorkflowStorage>> {
        // Use workflow-specific storage if configured
        if let Some(storage_config) = &workflow_def.storage {
            let cache_key = storage_config.cache_key();

            // Check cache first
            if let Some(storage) = self.storage_cache.read().await.get(&cache_key) {
                return Ok(storage.clone());
            }

            // Create new storage instance
            let credentials = self.secrets_manager
                .get_secret(&storage_config.credentials_secret)
                .await?;

            let storage = create_storage_backend(storage_config, credentials)?;

            // Cache for reuse
            self.storage_cache.write().await.insert(cache_key, storage.clone());

            Ok(storage)
        } else {
            // Fall back to system default storage
            Ok(self.default_storage.clone())
        }
    }
}
```

**Activity Context Enhancement**:
```rust
pub struct ActivityContext {
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub storage: Arc<dyn WorkflowStorage>,  // Per-workflow, not global
    // ...
}
```

**Benefits**:
- ✅ **Compliance**: Keep sensitive data in organization-controlled buckets
- ✅ **Cost optimization**: Avoid duplicate storage and cross-bucket transfer fees
- ✅ **Data sovereignty**: Ensure data stays in specific geographic regions
- ✅ **Integration**: Work directly with existing data lakes and warehouses
- ✅ **Multi-cloud**: Different workflows can use different cloud providers
- ✅ **No data migration**: Process existing data in place (reference_only mode)
- ✅ **Security**: Credentials isolated per workflow via Secrets Management

**Trade-offs**:
- Increased complexity (multiple storage backends active simultaneously)
- Credential management overhead (per-workflow credentials)
- User responsible for storage availability and lifecycle
- Potential performance variance (user's bucket might be slower/in different region)
- Testing complexity (validate multiple backend scenarios)

**Use Cases**:
1. **Regulatory compliance**: Healthcare/finance data must stay in specific buckets
2. **Data lake integration**: Process data in existing S3 data lake without copying
3. **Customer data isolation**: Multi-tenant SaaS with per-customer storage
4. **Cost optimization**: Large datasets processed in place (no transfer costs)
5. **Geographic requirements**: EU workflows use EU buckets, US workflows use US buckets

**Lifecycle Considerations**:
- **Reference-only mode**: StreamFlow doesn't manage lifecycle (user's responsibility)
- **Managed mode**: StreamFlow can still apply retention policies to workflow artifacts
- **Storage unavailability**: Workflow fails gracefully with clear error message
- **Credential rotation**: Support for credential updates without workflow redeployment

**Security Validation**:
- Workflow can only access storage specified in its definition
- Activities receive pre-configured storage instance (no credential access)
- Secret references validated against Secrets Management permissions
- Audit logging for storage access (which workflow accessed which bucket)

**Testing Strategy**:
- Unit tests: Storage factory caching and credential handling
- Integration tests: Multiple workflows with different storage backends simultaneously
- E2E tests: Workflow execution with external S3 bucket
- Security tests: Verify cross-workflow storage isolation
- Failure tests: Storage unavailable scenarios, credential errors

**Success Criteria**:
- ✅ Workflows can specify custom S3/GCS/Azure storage
- ✅ Multiple workflows with different storage backends run concurrently
- ✅ Storage instances cached and reused efficiently
- ✅ Credentials managed securely via Secrets Management
- ✅ Clear error messages when storage unavailable
- ✅ No performance regression vs system-default storage
- ✅ Complete documentation with examples

**Implementation Estimate**: 7-11 days
- Design & specification: 1-2 days
- Core implementation: 3-5 days
- Testing (multiple backends, credentials, fallback): 2-3 days
- Documentation: 1 day

**Rollout Plan**:
- **Phase 1**: Implement storage factory with system default fallback
- **Phase 2**: Add S3 per-workflow configuration (after Story 1.9 complete)
- **Phase 3**: Extend to GCS/Azure (leverage existing providers)
- **Phase 4**: Add reference_only mode for existing user data

---

### Story 1.12: Advanced File Handling Features

**Priority**: P2 (Medium - Enhanced file workflow capabilities)

**As** a workflow developer working with complex file processing
**I want** advanced file handling features beyond basic upload/download
**So that** I can build production-grade file processing pipelines

**Background**:
US-5.4 (Object Storage and File Management) delivered MVP file handling with:
- ✅ PostgreSQL Large Objects storage backend
- ✅ File output type (`type: file`)
- ✅ HTTP file download (GET with `download_to_file` parameter)
- ✅ FileExecutor temp directory management
- ✅ Automatic file upload/cleanup
- ✅ Template resolution (`{{FILE.activity.output}}`)

This story enhances file handling with production features deferred from the MVP.

**Scope**:

**1. HTTP File Upload via Multipart Form Data**
- POST requests with file attachments
- Multipart/form-data encoding
- Multiple files per request
- Mixed form fields and files
- Progress tracking for large uploads

**2. Folder Outputs**
- New output type: `type: folder`
- Activities can produce directories of files
- Template resolution: `{{FOLDER.activity.output}}`
- Recursive upload/download of directory trees
- Preserve directory structure
- Efficient handling of many small files

**3. File Versioning**
- Track file versions across workflow executions
- Version metadata (version number, timestamp, workflow_id)
- Retrieve specific file versions
- Compare versions (diff support)
- Retention policies for old versions

**4. File Compression**
- Automatic compression for large files (gzip, zstd)
- Compression level configuration per activity
- Transparent decompression on download
- Size reduction metrics
- CPU vs storage cost trade-offs

**5. File Encryption at Rest**
- Encrypt files before storage (AES-256)
- Key management integration
- Transparent decryption on download
- Per-workflow encryption keys
- Compliance support (HIPAA, PCI-DSS)

**6. Presigned URLs**
- Generate presigned URLs for direct client upload
- Bypass StreamFlow for large file transfers
- Configurable expiration (1 hour - 7 days)
- Security validation (workflow ownership)
- Works with S3, GCS, Azure Blob

**7. CDN Integration**
- Serve files via CDN for fast global access
- CloudFlare, Fastly, AWS CloudFront support
- Cache invalidation on file updates
- Geographic distribution
- Reduced latency for downloads

**8. Large File Performance**
- Validate streaming with files >100MB, >1GB, >10GB
- Chunked upload/download (multipart)
- Parallel chunk transfers
- Progress reporting
- Memory profiling (ensure no full file loads)
- Throughput benchmarks

**Dependencies**:
- ✅ US-5.4 (Object Storage) - base infrastructure
- Story 1.9 (S3 Storage) - for presigned URLs and CDN
- Secrets Management - for encryption keys

**Implementation Details**:

**HTTP Multipart Upload Example**:
```yaml
activities:
  - key: process_document
    worker: builtin
    activity_name: http_request
    parameters:
      method: POST
      url: "https://api.example.com/process"
      files:
        document: "{{FILE.download_doc.file}}"
        metadata: "{{FILE.prepare_metadata.json}}"
      form_data:
        user_id: "{{INPUT.user_id}}"
        priority: "high"
```

**Folder Output Example**:
```yaml
activities:
  - key: generate_reports
    worker: analytics
    activity_name: generate_monthly_reports
    outputs:
      - name: reports
        type: folder  # Directory of PDF files

  - key: upload_reports
    worker: builtin
    activity_name: s3_upload_folder
    parameters:
      folder: "{{FOLDER.generate_reports.reports}}"
      bucket: "company-reports"
      prefix: "monthly/{{INPUT.month}}"
```

**File Versioning API**:
```rust
// Get latest version
let file = storage.download_file(workflow_id, activity_key, filename).await?;

// Get specific version
let file_v2 = storage.download_file_version(workflow_id, activity_key, filename, 2).await?;

// List versions
let versions = storage.list_file_versions(workflow_id, activity_key, filename).await?;
```

**Presigned URL Example**:
```yaml
activities:
  - key: generate_upload_url
    worker: builtin
    activity_name: generate_presigned_url
    parameters:
      operation: put_object
      expiration_seconds: 3600
      filename: "user_document.pdf"
    outputs:
      - name: upload_url
        type: value

  # Client uploads directly to S3/GCS using presigned URL
  # No file passes through StreamFlow
```

**Benefits**:
- **Multipart upload**: Handle complex API integrations
- **Folder outputs**: Process entire datasets (images, logs, exports)
- **Versioning**: Debug issues, compare results, rollback
- **Compression**: Reduce storage costs by 60-90%
- **Encryption**: Meet compliance requirements
- **Presigned URLs**: Reduce StreamFlow bandwidth costs
- **CDN**: Fast global file access
- **Large file support**: Process TB-scale datasets

**Trade-offs**:
- Increased complexity in FileExecutor
- Additional storage metadata (versions, compression, encryption)
- Key management overhead (encryption)
- CDN integration complexity
- Testing scope expands significantly

**Success Criteria**:
- ✅ HTTP POST with multipart/form-data works
- ✅ Folder outputs upload/download correctly
- ✅ File versions tracked and retrievable
- ✅ Compression reduces storage by >50% for test files
- ✅ Encryption/decryption transparent to activities
- ✅ Presigned URLs work with S3/GCS
- ✅ Files >10GB stream correctly (no OOM)
- ✅ All features documented with examples

**Implementation Estimate**: 12-18 days
- HTTP multipart: 2-3 days
- Folder outputs: 3-4 days
- Versioning: 2-3 days
- Compression: 1-2 days
- Encryption: 2-3 days
- Presigned URLs: 1-2 days
- CDN integration: 2-3 days
- Large file testing: 1-2 days
- Documentation: 1 day

**Rollout Plan**:
- **Phase 1**: HTTP multipart upload (most requested)
- **Phase 2**: Folder outputs (enables dataset processing)
- **Phase 3**: Compression (storage cost reduction)
- **Phase 4**: Versioning (debugging and compliance)
- **Phase 5**: Encryption (security compliance)
- **Phase 6**: Presigned URLs & CDN (performance optimization)
- **Phase 7**: Large file validation (stress testing)

---

### Story 1.13: Redis Result Caching

**Priority**: P2 (Medium - Performance optimization)

**As** a workflow developer
**I want** StreamFlow to cache activity results in Redis
**So that** repeated executions with the same inputs are instant

**Scope**:
- Redis integration for activity results
- Cache key generation from activity inputs
- TTL configuration per activity type
- Cache invalidation strategies
- Cache hit/miss metrics
- Graceful degradation when Redis unavailable
- LRU eviction policies

**Architecture Reference**: `docs/architecture.md` (Redis optional for caching)

**Benefits**:
- 0ms execution time for cached results
- Reduced load on workers
- Cost savings for expensive activities (AI API calls)
- Improved workflow latency

**Configuration Example**:
```bash
STREAMFLOW_CACHE_PROVIDER=redis
STREAMFLOW_REDIS_URL=redis://localhost:6379
STREAMFLOW_CACHE_TTL_DEFAULT=3600  # 1 hour
```

---

### Story 1.14: Activity I/O Backends

**Priority**: P2 (Medium - Large data handling)

**As** a workflow developer
**I want** to configure where activity outputs are stored
**So that** large intermediate results don't bloat workflow state

**Background**: MVP stores all activity outputs in workflow state (PostgreSQL JSONB). This works well for small results but becomes problematic for:
- Large LLM responses (>100KB)
- File processing pipelines (images, PDFs, datasets)
- High-throughput workflows where state serialization becomes a bottleneck

**Scope**:
- Per-activity `output_store` setting: `state` (default), `redis`, `s3`, `postgres_lo`
- Transparent input resolution: `{{activity.result}}` fetches from configured backend
- TTL support for ephemeral data (Redis)
- Automatic cleanup on workflow completion
- Cross-workflow data sharing via explicit keys

**YAML Example**:
```yaml
activities:
  - key: generate_report
    activity_name: llm_prompt
    parameters:
      prompt: "Generate a detailed analysis..."
      max_tokens: 10000
    settings:
      output_store: redis      # Store large output in Redis
      output_ttl: 3600         # Auto-expire after 1 hour

  - key: process_report
    activity_name: some_processor
    depends_on: [generate_report]
    parameters:
      # Transparently fetched from Redis
      report: "{{generate_report.result.content}}"
```

**Backends**:
| Backend       | Use Case                          | TTL Support | Cross-Workflow |
|---------------|-----------------------------------|-------------|----------------|
| `state`       | Small results (<10KB), default    | No          | No             |
| `redis`       | Large ephemeral data, fast access | Yes         | Yes            |
| `s3`          | Very large files, persistence     | Yes         | Yes            |
| `postgres_lo` | Large objects, no external deps   | No          | No             |

**Benefits**:
- Workflow state stays small and fast
- Large data handled efficiently
- Flexible per-activity configuration
- No changes to activity code (transparent)

**Trade-offs**:
- Additional infrastructure (Redis/S3) for some backends
- Complexity in output resolution
- Cleanup coordination on workflow failure

**Related Stories**:
- Story 1.8: Redis Activity Queue (different - queue backend, not I/O)
- Story 1.13: Redis Result Caching (different - semantic caching, not storage)

---

### Story 1.15: Chat/Collaboration Notification Activities

**Priority**: P2 (Medium - Extends MVP email notifications to popular platforms)

**As** a platform engineering lead
**I want** built-in notification activities for Slack, Teams, and Discord
**So that** workflows can alert teams on their preferred collaboration platforms

**Scope**:
- `slack_message` - Post messages to Slack channels/users
- `teams_notify` - Send notifications to Microsoft Teams
- `discord_send` - Send messages to Discord channels
- `gchat_send` - Send messages to Google Chat spaces

**Slack Activity** (`slack_message`):
```yaml
activities:
  notify_team:
    activity: slack_message
    parameters:
      webhook_url: "{{SECRET.slack_webhook}}"
      channel: "#alerts"
      text: "Workflow {{WORKFLOW.id}} completed"
      blocks:  # Optional rich formatting
        - type: section
          text:
            type: mrkdwn
            text: "*Status:* Success"
      thread_ts: "{{INPUT.parent_message}}"  # Optional threading
```

**Teams Activity** (`teams_notify`):
```yaml
activities:
  notify_ops:
    activity: teams_notify
    parameters:
      webhook_url: "{{SECRET.teams_webhook}}"
      title: "Pipeline Complete"
      text: "Processed {{process.row_count}} records"
      theme_color: "00FF00"
      sections:
        - facts:
            - name: "Duration"
              value: "{{WORKFLOW.duration_ms}}ms"
```

**Discord Activity** (`discord_send`):
```yaml
activities:
  notify_discord:
    activity: discord_send
    parameters:
      webhook_url: "{{SECRET.discord_webhook}}"
      content: "Workflow completed!"
      embeds:
        - title: "Results"
          color: 0x00FF00
          fields:
            - name: "Records"
              value: "{{process.count}}"
```

**Benefits**:
- Native integration with popular collaboration tools
- Rich message formatting (blocks, cards, embeds)
- Thread/conversation support
- No external workers required

**Trade-offs**:
- Each platform has unique API requirements
- Webhook management complexity
- Rate limiting varies by platform
- OAuth flows for bot token mode (Slack)

**Implementation Notes**:
- Start with webhook-based integrations (simplest)
- Bot token mode (Slack) enables more features but requires OAuth
- Consider separate workers for each platform for isolation
- Rate limiting should be per-workspace/server

**Related Stories**:
- US-5.7 (MVP): `email_send` activity provides foundational notification support

---

## Epic 2: Performance Optimization

**Goal**: Achieve >10,000 workflows/sec throughput and <1ms orchestration latency through architectural optimizations.

### Story 2.1: Compiled Workflows

**Priority**: P1 (High - Major performance gain)

**As** a platform engineer scaling to thousands of workflows/sec
**I want** workflows to be pre-compiled into optimized execution plans
**So that** orchestration latency drops from ~1ms to <100μs

**Scope**:
- Workflow compilation at deployment time (not runtime)
- Pre-compute dependency graph (adjacency lists)
- Pre-evaluate static conditions
- Generate optimized evaluation code
- Cache compiled workflows in memory
- Invalidation on workflow definition updates
- Benchmark showing >10x evaluation speedup

**Architecture Reference**: `docs/architecture.md` (Performance Targets, "compiled workflow optimizations")

**Benefits**:
- 10x faster evaluation (<100μs vs ~1ms)
- Reduced CPU usage per workflow
- Higher throughput (>10,000 workflows/sec)

**Trade-offs**:
- Additional deployment step (compilation)
- Memory overhead for compiled workflows
- Complexity in invalidation logic

**Implementation Note**: This is mentioned in architecture.md as a post-MVP optimization. MVP uses runtime graph evaluation (~1ms latency).

---

### Story 2.2: Workflow State Caching

**Priority**: P1 (High - Reduces state reconstruction overhead)

**As** an orchestrator service
**I want** to cache reconstructed workflow state in memory
**So that** I don't rebuild state from events on every evaluation

**Scope**:
- LRU cache for active workflow states
- Configurable cache size (number of workflows)
- Cache invalidation on new events
- Cache warming strategies
- Cache hit/miss metrics
- Memory usage monitoring

**Architecture Reference**: `docs/implementation/US-1.2-event-driven-scheduling.md` (Future Enhancements)

**Benefits**:
- Faster evaluation (skip state reconstruction)
- Lower database load
- Better throughput for active workflows

**Configuration Example**:
```bash
STREAMFLOW_ORCHESTRATOR_STATE_CACHE_SIZE=10000  # Cache 10k active workflows
STREAMFLOW_ORCHESTRATOR_STATE_CACHE_TTL=300     # 5 minutes idle expiration
```

---

### Story 2.3: Event Table Partitioning for Long-Term Audit Retention

**Priority**: P1 (High - Critical for production auditability at scale)

**As** a platform engineer managing high event volume
**I want** the workflow_events table to be partitioned by time
**So that** queries remain fast while retaining complete audit history indefinitely

**Background**:
- **workflow_events must NEVER be deleted** - provides complete audit trail for compliance
- Unlike workflows/activity_queue (cleaned up via TTL), events are kept forever
- As event volume grows, query performance degrades without partitioning
- Partitioning enables fast queries while preserving all historical data

**Scope**:
- Time-based partitioning (monthly recommended for audit data)
- Automated partition management (pg_partman or custom Rust tool)
- Partition creation ahead of time (auto-create next 3 months)
- **NO partition deletion** - old partitions retained forever (audit requirement)
- Partition archival to cold storage (optional - move old partitions to archive tablespace)
- Index management per partition
- Query optimization for partitioned tables (partition pruning)
- Monitoring partition sizes and query performance

**Architecture Reference**:
- `docs/implementation/US-1.2-event-driven-scheduling.md` (Future Enhancements: Event Table Partitioning)
- `docs/2025-11-14-analysis-absurd-orchestrator.md` Section 5 (Cleanup Strategy)

**Benefits**:
- **Faster queries** - Partition pruning limits scans to relevant time ranges
- **Scalable audit retention** - Keep all events forever without performance degradation
- **Better vacuum performance** - VACUUM scoped to recent partitions
- **Index maintenance** - Smaller indexes per partition, faster rebuilds
- **Optional cold storage** - Archive old partitions to cheaper storage
- **Compliance ready** - Complete audit trail with efficient access

**Implementation**:
```sql
-- Convert workflow_events to partitioned table (requires downtime or migration)
CREATE TABLE workflow_events_new (
    id UUID NOT NULL,
    workflow_id UUID NOT NULL,
    event_type TEXT NOT NULL,
    activity_key TEXT,
    payload JSONB NOT NULL,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, timestamp)  -- Timestamp must be in primary key for partitioning
) PARTITION BY RANGE (timestamp);

-- Create initial partitions (monthly)
CREATE TABLE workflow_events_2025_11 PARTITION OF workflow_events_new
FOR VALUES FROM ('2025-11-01') TO ('2025-12-01');

CREATE TABLE workflow_events_2025_12 PARTITION OF workflow_events_new
FOR VALUES FROM ('2025-12-01') TO ('2026-01-01');

CREATE TABLE workflow_events_2026_01 PARTITION OF workflow_events_new
FOR VALUES FROM ('2026-01-01') TO ('2026-02-01');

-- Indexes on partitioned table (automatically created on each partition)
CREATE INDEX idx_workflow_events_workflow_id
ON workflow_events_new(workflow_id, timestamp DESC);

CREATE INDEX idx_workflow_events_type
ON workflow_events_new(event_type, timestamp DESC);

-- Automated partition management (Rust background task)
-- Creates new partitions 3 months in advance
-- Monitors partition sizes and query performance
```

**Partition Management Tool**:
```rust
// Background task to auto-create partitions
pub async fn manage_event_partitions(pool: &PgPool) -> Result<()> {
    let next_months = get_upcoming_months(3); // Next 3 months

    for month in next_months {
        let partition_name = format!("workflow_events_{}", month.format("%Y_%m"));
        let start_date = month.format("%Y-%m-01");
        let end_date = (month + Duration::days(32)).format("%Y-%m-01"); // Next month

        // Create partition if doesn't exist
        sqlx::query(&format!(
            "CREATE TABLE IF NOT EXISTS {} PARTITION OF workflow_events
             FOR VALUES FROM ('{}') TO ('{}')",
            partition_name, start_date, end_date
        ))
        .execute(pool)
        .await?;

        info!("Ensured partition {} exists", partition_name);
    }

    Ok(())
}
```

**Migration Strategy**:
1. **Phase 1**: Enable partitioning on new deployment (before MVP launch)
2. **Phase 2** (If converting existing table):
   - Create new partitioned table with `_new` suffix
   - Copy data in batches with time-based filtering
   - Minimal downtime swap (rename tables atomically)
   - Validate data integrity
3. **Automation**: Deploy partition management background task
4. **Monitoring**: Track partition sizes, query performance, disk usage

**Query Performance**:
```sql
-- Query with partition pruning (fast - scans only November 2025 partition)
SELECT * FROM workflow_events
WHERE workflow_id = $1
  AND timestamp >= '2025-11-01'
  AND timestamp < '2025-12-01';

-- Without partition pruning (slower - scans all partitions)
SELECT * FROM workflow_events
WHERE workflow_id = $1;  -- No timestamp filter
```

**Archival Strategy** (Optional):
- Move partitions >1 year old to archive tablespace (cheaper storage)
- Keep indexes on archive partitions (queries still work, just slower)
- Never delete partitions (audit requirement)

**Monitoring**:
- Partition count (should grow monthly)
- Partition sizes (MB per partition)
- Query performance by time range (P95 latency)
- Disk usage trend (should be linear growth)

**Trade-offs**:
- Migration complexity (converting existing table)
- Timestamp must be in primary key (schema constraint)
- Queries without timestamp filter slower (full table scan)
- Storage grows indefinitely (mitigated by compression/archival)

**Success Criteria**:
- Queries with timestamp filter <100ms P95 (even with billions of events)
- Partition creation automated (no manual intervention)
- Complete audit trail preserved (zero data loss)
- Disk usage predictable and linear

---

### Story 2.4: PostgreSQL Activity Pool Cache Enhancements

**Priority**: P3 (Lower - Optimization for specific edge cases)

**As** a platform engineer running the postgres_query built-in activity
**I want** advanced pool caching features (eviction, cache metrics)
**So that** I can optimize connection management for edge cases

**Background**:
The postgres_query built-in activity caches PostgreSQL connection pools by database URL to avoid connection overhead. MVP implementation uses a simple cache with no eviction. This works well for typical workflows (connecting to 1-3 databases), but edge cases may benefit from advanced cache management.

**Current MVP Implementation**:
- ✅ Connection pool cache (HashMap by db_url)
- ✅ System-level pool configuration via environment variables
- ✅ Reasonable defaults (max_connections=5, acquire_timeout=30s)
- ✅ Environment variables:
  - `STREAMFLOW_POSTGRES_POOL_MAX_CONNECTIONS` (default: 5)
  - `STREAMFLOW_POSTGRES_POOL_MIN_CONNECTIONS` (default: none)
  - `STREAMFLOW_POSTGRES_POOL_ACQUIRE_TIMEOUT_SECS` (default: 30)
  - `STREAMFLOW_POSTGRES_POOL_MAX_LIFETIME_SECS` (default: none)
  - `STREAMFLOW_POSTGRES_POOL_IDLE_TIMEOUT_SECS` (default: none)
- ❌ No cache eviction (pools live forever)
- ❌ No cache size limits
- ❌ No cache metrics

**Scope**:
- **Automatic cache eviction**: Evict pools not used in last N minutes
- **Configurable eviction interval**: Background task frequency (default: 5 minutes)
- **Configurable idle threshold**: Evict pools idle for N minutes (default: 10 minutes)
- **Cache size limits**: Maximum number of cached pools (default: unlimited)
- **Cache metrics**: Pool count, eviction count, cache hit/miss rate

**Configuration Example**:
```bash
# PostgreSQL activity pool settings (MVP - already supported)
STREAMFLOW_POSTGRES_POOL_MAX_CONNECTIONS=10            # Default: 5
STREAMFLOW_POSTGRES_POOL_ACQUIRE_TIMEOUT_SECS=60      # Default: 30

# PostgreSQL activity pool cache settings (Post-MVP)
STREAMFLOW_POSTGRES_POOL_CACHE_MAX_SIZE=100            # Default: unlimited
STREAMFLOW_POSTGRES_POOL_EVICTION_INTERVAL_SECS=300    # 5 minutes
STREAMFLOW_POSTGRES_POOL_IDLE_THRESHOLD_SECS=600       # 10 minutes
```

**Eviction Logic**:
```rust
// Background task runs every eviction_interval
pub async fn evict_idle_pools(
    pool_cache: Arc<RwLock<HashMap<String, (PgPool, Instant)>>>,
    idle_threshold: Duration,
) {
    let mut cache = pool_cache.write().await;
    let now = Instant::now();

    cache.retain(|db_url, (pool, last_used)| {
        let age = now.duration_since(*last_used);
        if age > idle_threshold {
            info!("Evicting idle pool for {} (idle for {:?})", db_url, age);
            false  // Remove from cache
        } else {
            true  // Keep in cache
        }
    });
}
```

**Benefits**:
- ✅ Prevents unbounded cache growth
- ✅ Releases unused database connections
- ✅ Configurable for different use cases
- ✅ Metrics for monitoring cache behavior

**Trade-offs**:
- Adds complexity (background eviction task)
- Pool recreation overhead if evicted too aggressively
- Configuration tuning required for optimal behavior

**Testing**:
- Unit test: Eviction logic removes idle pools
- Integration test: Verify pool reuse within idle threshold
- Integration test: Verify pool eviction after idle threshold
- Load test: Cache behavior under high pool churn

**Success Criteria**:
- ✅ Idle pools evicted within 2x eviction interval
- ✅ Cache size stays below max_size limit
- ✅ Metrics show cache hit rate >90% for typical workflows
- ✅ No performance regression vs MVP (simple cache)

**When to Use**:
- ✅ Workflows connecting to many ephemeral databases (>10)
- ✅ Multi-tenant SaaS with per-tenant databases
- ✅ Memory-constrained environments
- ❌ Typical workflows connecting to 1-3 databases (MVP cache sufficient)

**Architecture Reference**:
- `worker/src/activities/postgres.rs:20` (PoolConfig with environment variables)
- `worker/src/activities/postgres.rs:75` (get_pool method with cache logic)

---

### Story 2.5: Activity Queue Partitioning

**Priority**: P3 (Lower - Only needed at very high scale)

**As** a platform engineer managing >50,000 activities/sec
**I want** the activity queue to be partitioned by worker or workflow_id
**So that** queue operations scale horizontally

**Scope**:
- Partition by worker (different activity types)
- Partition by workflow_id hash (distribute load)
- Partition-aware worker polling
- Query routing to correct partition
- Monitoring per partition

**Benefits**:
- Better concurrency (reduce lock contention)
- Higher throughput
- Easier to scale specific activity types

**Trade-offs**:
- Increased complexity
- Partition management overhead

---

### Story 2.6: Priority Queues

**Priority**: P2 (Medium - Common requirement for production)

**As** a workflow developer
**I want** to assign priorities to activities
**So that** critical workflows execute before background jobs

**Background**:
Currently, StreamFlow's activity queue processes activities in FIFO order (by `scheduled_for`). This means high-priority interactive workflows can be blocked behind low-priority batch jobs, causing SLA violations.

**Scope**:
- Add `priority` field to activity_queue table
- Integer priority: 1 (highest) to 9 (lowest), default 5 (normal)
- Index on (worker, name, priority DESC, scheduled_for ASC)
- Update claim_next() to order by priority first, then scheduled_for
- Starvation prevention (age-based boost after threshold)
- Priority configuration in YAML workflow definitions
- Metrics: queue depth by priority level

**Schema**:
```sql
ALTER TABLE activity_queue ADD COLUMN priority INTEGER NOT NULL DEFAULT 5;

-- New index for priority-aware polling
CREATE INDEX idx_queue_priority
ON activity_queue(worker, name, priority DESC, scheduled_for ASC)
WHERE status = 'pending';

-- Drop old index (replaced by priority-aware version)
DROP INDEX IF EXISTS idx_queue_polling;
```

**claim_next() Update**:
```sql
-- Updated query in claim_next() stored procedure
SELECT id, workflow_id, activity_key, worker, activity_name, parameters, settings
FROM activity_queue
WHERE status = 'pending'
  AND scheduled_for <= NOW()
  AND worker = $1
  AND activity_name = $2
ORDER BY priority ASC,      -- Lower number = higher priority
         scheduled_for ASC  -- FIFO within same priority
LIMIT 1
FOR UPDATE SKIP LOCKED;
```

**YAML Configuration** (Workflow Definition):
```yaml
activities:
  # Critical real-time payment processing
  - key: authorize_payment
    worker: payments
    name: charge_card
    parameters:
      card_token: "{{INPUT.card_token}}"
      amount: "{{INPUT.amount}}"
    settings:
      priority: 1  # Highest priority - execute immediately
      timeout_seconds: 30

  # Normal priority notification
  - key: send_confirmation
    worker: notifications
    name: send_email
    parameters:
      recipient: "{{INPUT.email}}"
    settings:
      priority: 5  # Normal priority (default)
      timeout_seconds: 60

  # Low priority batch report
  - key: generate_analytics
    worker: analytics
    name: daily_summary
    parameters:
      date: "{{WORKFLOW.created_at}}"
    settings:
      priority: 9  # Lowest priority - runs when idle
      timeout_seconds: 3600
```

**Starvation Prevention**:
```rust
// After activity sits in queue for threshold, boost priority
// Prevents low-priority activities from starving indefinitely
const STARVATION_THRESHOLD: Duration = Duration::from_secs(600); // 10 minutes
const PRIORITY_BOOST: i32 = 2;  // Boost by 2 levels

// In claim_next() logic
if activity.scheduled_for + STARVATION_THRESHOLD < now() {
    effective_priority = max(1, activity.priority - PRIORITY_BOOST);
}
```

**Benefits**:
- ✅ SLA guarantees for critical workflows (payments, user-facing operations)
- ✅ Batch jobs don't block interactive workflows
- ✅ Better resource utilization (high-value work first)
- ✅ Fairness with starvation prevention (age-based boost)
- ✅ Simple integer priority (1-9 scale, easy to understand)
- ✅ No need for separate queue tables (single table with priority column)
- ✅ Metrics: Monitor queue depth by priority level

**Trade-offs**:
- Adds complexity to queue polling logic
- Index size increases (priority column added)
- Starvation prevention requires periodic boost calculation
- Priority decisions require workflow design consideration

**Testing**:
- Unit test: Priority ordering in claim_next()
- Unit test: Starvation prevention boost
- Integration test: High-priority activities execute before low-priority
- Load test: Verify no starvation under high load
- Metrics test: Queue depth by priority tracked correctly

**Success Criteria**:
- ✅ Priority 1 activities execute within 1 second (P95) under normal load
- ✅ No activity waits >10 minutes regardless of priority (starvation prevention)
- ✅ Queue depth metrics show distribution by priority level
- ✅ Workflow definitions can specify priority per activity
- ✅ Default priority 5 applied when not specified

**Architecture Reference**:
- `docs/2025-11-14-analysis-absurd-orchestrator.md` Section 10 (Multiple Queue Support)

---

### Story 2.7: Dead Letter Queue

**Priority**: P2 (Medium - Improves debuggability)

**As** a platform engineer troubleshooting failures
**I want** permanently failed activities moved to a dead letter queue
**So that** I can analyze failures without losing data

**Scope**:
- Separate `activity_failures` table
- Move failed activities (max retries exceeded) to DLQ
- Retention policies (days/count)
- API to query DLQ
- Retry from DLQ (manual recovery)
- Metrics on failure types

**Schema**:
```sql
CREATE TABLE activity_failures (
    id UUID PRIMARY KEY,
    original_activity_id UUID NOT NULL,
    workflow_id UUID NOT NULL,
    activity_key TEXT NOT NULL,
    worker TEXT NOT NULL,
    name TEXT NOT NULL,
    parameters JSONB NOT NULL,
    error TEXT NOT NULL,
    retry_count INTEGER NOT NULL,
    failed_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);
```

**Benefits**:
- Failure analysis and debugging
- Manual recovery paths
- Metrics on failure patterns
- Data retention for compliance

---

## Epic 3: Multi-Tenancy & Authorization

**Goal**: Support multiple tenants in a single StreamFlow deployment with proper isolation and access control.

### Story 3.1: Row-Level Security for Multi-Tenancy

**Priority**: P1 (High - Key enterprise feature)

**As** a SaaS provider using StreamFlow
**I want** to deploy a single StreamFlow instance for all customers
**So that** I reduce operational overhead while ensuring data isolation

**Scope**:
- Add `tenant_id` column to all tables (workflows, activity_queue, workflow_events, etc.)
- PostgreSQL Row-Level Security (RLS) policies
- Tenant-scoped queries in all APIs
- JWT claims include tenant_id
- Tenant creation/management APIs
- Per-tenant resource limits (workflows, activities, storage)
- Cross-tenant access prevention (security testing)

**Architecture Reference**: `docs/architecture.md` (Multi-Tenancy section)

**Schema Changes**:
```sql
ALTER TABLE workflows ADD COLUMN tenant_id UUID NOT NULL;
ALTER TABLE activity_queue ADD COLUMN tenant_id UUID NOT NULL;
ALTER TABLE workflow_events ADD COLUMN tenant_id UUID NOT NULL;

-- RLS policies
ALTER TABLE workflows ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation ON workflows
USING (tenant_id = current_setting('app.tenant_id')::UUID);
```

**Benefits**:
- Single deployment for all customers
- Lower operational overhead
- Database-enforced isolation
- Cost savings vs per-tenant deployments

---

### Story 3.2: Role-Based Access Control (RBAC)

**Priority**: P2 (Medium - Common requirement)

**As** a platform administrator
**I want** to assign different permissions to users (admin, developer, viewer)
**So that** I can control who can create, modify, or view workflows

**Scope**:
- Define roles (admin, developer, viewer)
- Permission model (create_workflow, delete_workflow, view_workflow, execute_activity, etc.)
- Role assignment to users/clients
- Permission checks in API endpoints
- Audit logging for access decisions
- JWT claims include roles/permissions

**Permission Model**:
- `workflow:read` - View workflow definitions and status
- `workflow:write` - Create/update workflow definitions
- `workflow:execute` - Start workflow instances
- `workflow:delete` - Delete workflows
- `activity:read` - View activity status
- `activity:execute` - Execute activities (for workers)
- `admin:*` - Full administrative access

**Benefits**:
- Fine-grained access control
- Separation of duties
- Audit trail for compliance
- Prevent unauthorized actions

---

### Story 3.3: Per-Tenant Resource Limits

**Priority**: P2 (Medium - Prevent resource exhaustion)

**As** a SaaS provider
**I want** to enforce resource limits per tenant (max workflows, max storage)
**So that** one tenant cannot exhaust shared resources

**Scope**:
- Configurable limits per tenant (workflows, activities, storage, events)
- Real-time quota tracking (counters)
- Reject requests exceeding quota (HTTP 429 Too Many Requests)
- Quota metrics and alerts
- Admin APIs to view/update quotas
- Grace periods and soft limits

**Quotas**:
- Max concurrent workflows
- Max activities per workflow
- Max storage per tenant (artifacts)
- Max events per time window
- Max API requests per minute

**Benefits**:
- Fair resource sharing
- Prevent noisy neighbor problems
- Predictable performance
- Revenue optimization (tiered limits)

---

## Epic 4: Developer Experience

**Goal**: Improve developer experience with better tools, SDKs, and workflow definition capabilities.

### Story 4.1: Python SDK for Workflow Definitions

**Priority**: P1 (High - Python is primary AI/ML language)

**As** a Python developer
**I want** to define workflows programmatically in Python
**So that** I get type safety, IDE autocomplete, and reusable components

**Scope**:
- Python library for workflow definitions
- Compile to YAML at deployment time
- Type hints for parameters and outputs
- Validation at definition time
- Unit testing support for workflow logic
- Integration with popular IDEs (VSCode, PyCharm)
- Documentation and examples

**Architecture Reference**: `docs/architecture.md` (Programmatic workflow definitions)

**Example API**:
```python
from streamflow import Workflow, Activity

workflow = Workflow("payment_processing", version="1.0")

validate = Activity(
    key="validate_payment",
    worker="payments",
    name="validate_card",
    parameters={"card_token": workflow.arg("card_token")}
)

authorize = Activity(
    key="authorize_card",
    worker="payments",
    name="authorize"
).with_preceding(validate).when(validate.outputs.valid == True)

workflow.add_activities(validate, authorize)
workflow.compile()  # Generates YAML
```

**Benefits**:
- Type safety (catch errors early)
- IDE support (autocomplete, refactoring)
- Reusable components (workflow libraries)
- Better testing (unit test workflow logic)

---

### Story 4.2: TypeScript SDK for Workflow Definitions

**Priority**: P2 (Medium - JavaScript/TypeScript common)

**As** a TypeScript developer
**I want** to define workflows programmatically in TypeScript
**So that** I get type safety in my Node.js projects

**Scope**:
- Similar to Python SDK but for TypeScript
- Compile to YAML at build time
- Type definitions for activities
- Integration with popular frameworks (Express, NestJS)

---

### Story 4.3: Rust SDK for Workflow Definitions

**Priority**: P3 (Lower - Advanced use cases)

**As** a Rust developer building high-performance workflows
**I want** to define workflows in Rust
**So that** I get compile-time guarantees and zero-cost abstractions

**Scope**:
- Rust library for workflow definitions
- Proc macros for ergonomic API
- Compile to YAML at build time
- Integration with StreamFlow core types

---

### Story 4.4: Complex Expression Language

**Priority**: P2 (Medium - Enables advanced workflows)

**As** a workflow developer
**I want** to use rich expressions in conditions and parameters
**So that** I can implement complex business logic without custom activities

**Scope**:
- Expression parser library (evalexpr, rhai, or custom)
- Arithmetic operations (+, -, *, /, %)
- String operations (concat, substring, length, contains)
- Array operations (map, filter, contains, length)
- Object operations (get, has, keys)
- Function calls (now(), random(), uuid(), etc.)
- Type coercion and validation
- Error handling for invalid expressions

**Architecture Reference**: `docs/implementation/US-1.2-event-driven-scheduling.md` (Future Enhancements: Complex Condition Expressions)

**Example Expressions**:
```yaml
conditions:
  # Arithmetic
  - "{{order.total}} > 1000"
  - "{{user.age}} >= 18 && {{user.verified}} == true"

  # String operations
  - "{{user.email}}.contains('@example.com')"
  - "{{product.name}}.length() > 0"

  # Array operations
  - "{{items}}.length() > 0"
  - "{{tags}}.contains('urgent')"

  # Function calls
  - "{{order.created_at}} < now() - duration('24h')"
```

**Benefits**:
- Richer workflow logic without code
- Reduce need for custom activities
- More expressive conditions
- Better developer experience

---

### Story 4.5: Web-Based Workflow Designer

**Priority**: P2 (Medium - Non-technical users)

**As** a business analyst
**I want** to design workflows visually in a web UI
**So that** I can create workflows without writing YAML or code

**Scope**:
- Drag-and-drop workflow designer
- Visual representation of directed graph
- Activity palette (worker/name browser)
- Parameter editor with validation
- Condition builder (visual expression editor)
- Real-time YAML preview
- Save/deploy workflows via API
- Version management

**Benefits**:
- Lower barrier to entry
- Non-technical users can create workflows
- Visual debugging (see workflow structure)
- Faster iteration

---

### Story 4.6: CLI for Workflow Management

**Priority**: P2 (Medium - Developer productivity)

**As** a developer
**I want** a CLI tool for StreamFlow operations
**So that** I can manage workflows, view logs, and debug from the terminal

**Scope**:
- `streamflow` CLI tool
- Commands:
  - `streamflow workflow deploy <file>` - Deploy workflow definition
  - `streamflow workflow list` - List deployed workflows
  - `streamflow workflow run <name>` - Start workflow instance
  - `streamflow workflow status <id>` - Check workflow status
  - `streamflow workflow logs <id>` - View workflow events
  - `streamflow activity list` - View queued activities
  - `streamflow admin create-client` - Create worker client
  - `streamflow admin create-user` - Create user account
- Shell completion (bash, zsh, fish)
- JSON output mode for scripting

**Benefits**:
- Developer productivity
- Scriptable operations (CI/CD)
- Quick debugging and troubleshooting
- Better DX vs raw HTTP APIs

---

### Story 4.7: Activity Development Kit

**Priority**: P2 (Medium - Simplify activity development)

**As** an activity developer
**I want** a standardized template for implementing activities
**So that** I can focus on business logic, not boilerplate

**Scope**:
- Activity SDK for common languages (Python, TypeScript, Rust)
- Standardized structure (parameters, outputs, errors)
- Built-in authentication (obtain token, refresh)
- Built-in polling loop (claim, execute, complete)
- Built-in heartbeat management
- Logging and metrics helpers
- Testing utilities (mock StreamFlow API)
- Documentation and examples

**Example Python Activity**:
```python
from streamflow.activity import Activity, activity, Input, Output

@activity(worker="payments", name="validate_card")
class ValidateCardActivity(Activity):
    class Parameters(Input):
        card_token: str

    class Outputs(Output):
        valid: bool
        error: str | None

    def execute(self, params: Parameters) -> Outputs:
        # Business logic here
        return Outputs(valid=True, error=None)
```

**Benefits**:
- Faster activity development
- Consistent structure across activities
- Less boilerplate
- Easier testing

---

### Story 4.8: Workflow Testing Framework

**Priority**: P2 (Medium - Quality assurance)

**As** a workflow developer
**I want** to unit test my workflows locally
**So that** I can verify behavior before deployment

**Scope**:
- Test framework for workflows (Python/TypeScript)
- Mock activity implementations
- Workflow simulation (step-through execution)
- Assertions on workflow state
- Test fixtures for common scenarios
- Integration with pytest/jest

**Example**:
```python
from streamflow.testing import WorkflowTest, mock_activity

class TestPaymentWorkflow(WorkflowTest):
    def test_successful_payment(self):
        # Mock activities
        self.mock_activity("payments.validate_card", outputs={"valid": True})
        self.mock_activity("payments.authorize", outputs={"auth_id": "123"})

        # Run workflow
        result = self.run_workflow("payment_processing", args={"card_token": "tok_123"})

        # Assertions
        assert result.status == "completed"
        assert result.activities["authorize_card"].outputs["auth_id"] == "123"
```

**Benefits**:
- Catch bugs before deployment
- Faster iteration (no deployment needed)
- Regression testing
- Better code quality

---

### Story 4.9: SQLite Support as Artifact-Based Activities

**Priority**: P3 (Lower - Specialized use cases)

**As** a workflow developer
**I want** to work with SQLite databases in my workflows
**So that** I can process, transform, and analyze data stored in SQLite files

**Challenge**: SQLite is file-based, not server-based, which creates a fundamental mismatch with StreamFlow's distributed worker architecture:
- Workers may run on different machines
- No server/client architecture to connect to
- Concurrent access requires file system coordination

**Approach**: Treat SQLite databases as **artifacts** that flow through workflows rather than as shared database connections:

1. **SQLite as Input Artifact**: Activity downloads SQLite file from WorkflowStorage, operates on it locally
2. **SQLite as Output Artifact**: Activity uploads modified SQLite file back to WorkflowStorage
3. **Sequential Access**: Workflow design ensures only one activity operates on a given SQLite file at a time

**Why NOT a `sqlite_query` activity**:
- Unlike `database_query` (PostgreSQL, MySQL), there's no server to connect to
- Connection pooling and caching don't apply
- File must be local to the worker executing the query

**Proposed Activities**:

```yaml
# Create a new SQLite database
- key: create_db
  worker: builtin
  name: sqlite_create
  parameters:
    schema: |
      CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);
      CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER);
  outputs:
    database: "{{sqlite_artifact}}"  # Artifact reference

# Execute queries on an existing SQLite database
- key: process_data
  worker: builtin
  name: sqlite_execute
  parameters:
    database: "{{create_db.outputs.database}}"
    queries:
      - "INSERT INTO users (name) VALUES ('Alice')"
      - "INSERT INTO users (name) VALUES ('Bob')"
  depends_on: [create_db]
  outputs:
    database: "{{sqlite_artifact}}"

# Query and return results
- key: get_results
  worker: builtin
  name: sqlite_query
  parameters:
    database: "{{process_data.outputs.database}}"
    query: "SELECT * FROM users"
  depends_on: [process_data]
  outputs:
    rows: "{{query_results}}"
```

**Alternative Approaches Considered**:

| Approach                      | Pros                               | Cons                                               |
|-------------------------------|------------------------------------|----------------------------------------------------|
| **Artifact-based (chosen)**   | Aligns with SQLite design, simple  | Sequential access only, file transfer overhead     |
| Worker-local SQLite           | Fast local access                  | Can't share data between activities                |
| Shared filesystem (NFS/EFS)   | Appears like local file            | Performance issues, locking contention, fragile    |
| rqlite/LiteFS                 | Server-like API                    | Additional infrastructure, defeats SQLite simplicity|

**Use Cases**:
- Data transformation pipelines (ingest CSV → SQLite → process → export)
- Offline analytics processing
- Report generation workflows
- ETL workflows with intermediate storage
- Testing/dev workflows with embedded data

**Benefits**:
- Works with SQLite's design (single-writer, file-based)
- No additional infrastructure required
- Leverages existing WorkflowStorage for artifact persistence
- Clear ownership semantics (one activity at a time)

**Trade-offs**:
- File transfer overhead for each activity (mitigated by WorkflowStorage caching)
- No concurrent access patterns
- Requires careful workflow design for data dependencies

---

## Epic 5: Enterprise Operations

**Goal**: Enable StreamFlow to run reliably in production with monitoring, high availability, and disaster recovery.

### Story 5.1: Metrics and Monitoring

**Priority**: P1 (High - Essential for production)

**As** a platform engineer running StreamFlow in production
**I want** comprehensive metrics exposed via Prometheus
**So that** I can monitor system health and performance

**Scope**:
- Prometheus metrics endpoint (`/metrics`)
- Core metrics:
  - Workflow throughput (workflows/sec)
  - Activity throughput (activities/sec)
  - Orchestration latency (P50, P95, P99)
  - Queue depth (pending activities by worker/name)
  - Event polling latency
  - Worker count (active workers)
  - Database connection pool utilization
  - Error rates (by error type)
  - **Activity scheduling metrics**:
    - Scheduled vs immediate activities (ratio)
    - Scheduling delay distribution (P50, P95, P99 of delay duration)
    - Activities scheduled in the past (warning count)
- Custom metrics support
- Grafana dashboards (pre-built)
- Alerting rules (Prometheus AlertManager)

**Key Metrics**:
```
streamflow_workflows_total{status="completed|failed"}
streamflow_workflow_duration_seconds{quantile="0.5|0.95|0.99"}
streamflow_activities_total{worker,name,status}
streamflow_activity_duration_seconds{worker,name,quantile}
streamflow_queue_depth{worker,name}
streamflow_orchestrator_evaluation_duration_seconds{quantile}
streamflow_db_connections{state="idle|active"}
streamflow_activities_scheduled_total{type="immediate|delayed"}
streamflow_activity_schedule_delay_seconds{quantile="0.5|0.95|0.99"}
streamflow_activities_scheduled_past_total
```

**Benefits**:
- Real-time visibility into system health
- Performance troubleshooting
- Capacity planning
- SLA monitoring

---

### Story 5.2: Distributed Tracing

**Priority**: P2 (Medium - Advanced debugging)

**As** a platform engineer troubleshooting slow workflows
**I want** distributed tracing across all StreamFlow components
**So that** I can identify performance bottlenecks

**Scope**:
- OpenTelemetry integration
- Trace spans for:
  - Workflow execution (start to completion)
  - Activity execution (queue to completion)
  - Orchestrator evaluation
  - Database queries
  - External API calls
- Trace context propagation (via headers)
- Integration with Jaeger/Zipkin/Datadog
- Sampling strategies (avoid overhead)

**Benefits**:
- Visualize workflow execution paths
- Identify slow activities/queries
- Correlate logs and traces
- Better debugging for complex workflows

---

### Story 5.3: Structured Logging

**Priority**: P2 (Medium - Production debugging)

**As** a platform engineer investigating issues
**I want** structured JSON logs with trace IDs
**So that** I can correlate logs across components

**Scope**:
- JSON log format (structured)
- Log levels (debug, info, warn, error)
- Trace ID in all logs (correlate with tracing)
- Workflow ID and activity ID in relevant logs
- Configurable log output (stdout, file, syslog)
- Integration with log aggregation (ELK, Datadog, CloudWatch)
- PII redaction (automatic scrubbing)

**Log Format**:
```json
{
  "timestamp": "2025-10-29T12:34:56.789Z",
  "level": "info",
  "message": "Activity completed successfully",
  "trace_id": "abc123",
  "workflow_id": "wf_xyz789",
  "activity_id": "act_456",
  "worker": "payments",
  "name": "authorize_card",
  "duration_ms": 123
}
```

**Benefits**:
- Machine-parseable logs
- Easy filtering and searching
- Correlation with traces
- Better production debugging

---

### Story 5.4: High Availability (HA) Setup

**Priority**: P1 (High - Production requirement)

**As** a platform engineer running StreamFlow in production
**I want** StreamFlow to handle component failures gracefully
**So that** workflows continue executing during partial outages

**Scope**:
- Multiple orchestrator instances (automatic failover)
- Multiple API server instances (load balancing)
- Multiple worker instances (automatic redistribution)
- PostgreSQL high availability (replication, failover)
- Health check endpoints for load balancers
- Graceful shutdown (drain connections, finish in-flight work)
- Circuit breakers for external dependencies
- Retry policies with exponential backoff

**Deployment Topology**:
```
Load Balancer
├─ API Server 1
├─ API Server 2
└─ API Server 3

Orchestrator Pool
├─ Orchestrator 1
├─ Orchestrator 2
└─ Orchestrator 3

Worker Pool
├─ Worker 1 (worker: payments)
├─ Worker 2 (worker: payments)
├─ Worker 3 (worker: data)
└─ Worker 4 (worker: data)

PostgreSQL
├─ Primary (writes)
└─ Replicas (reads, failover)
```

**Benefits**:
- No single point of failure
- Minimal downtime during failures
- Automatic recovery
- Better SLA guarantees

---

### Story 5.5: Disaster Recovery

**Priority**: P2 (Medium - Business continuity)

**As** a platform engineer
**I want** to recover from catastrophic failures (datacenter loss)
**So that** we can resume operations with minimal data loss

**Scope**:
- Automated backups (PostgreSQL, configs)
- Backup retention policies (7 days, 4 weeks, 12 months)
- Point-in-time recovery (PITR)
- Cross-region replication (PostgreSQL)
- Disaster recovery runbooks
- RTO/RPO targets (Recovery Time/Point Objectives)
- Regular DR drills (test recovery procedures)

**Backup Strategy**:
- Continuous WAL archiving (PostgreSQL)
- Daily full backups
- Hourly incremental backups
- Backup verification (restore test)

**Benefits**:
- Recover from catastrophic failures
- Minimize data loss (RPO)
- Minimize downtime (RTO)
- Business continuity assurance

---

### Story 5.6: Configuration Management

**Priority**: P2 (Medium - Operational simplicity)

**As** a platform engineer
**I want** to manage StreamFlow configuration centrally
**So that** I can update settings without redeploying

**Scope**:
- Configuration database table (key-value store)
- Hot reload of configuration (no restart required)
- Configuration versioning (track changes)
- Configuration validation (prevent invalid values)
- Audit logging for configuration changes
- Admin API for configuration management
- Environment-specific overrides

**Configurable Settings**:
- Queue polling intervals
- Orchestrator evaluation timeout
- Worker concurrency limits
- Retry policies (max retries, backoff)
- Resource quotas per tenant

**Benefits**:
- Dynamic configuration updates
- No deployment for config changes
- Audit trail for changes
- Environment parity (dev/staging/prod)

---

### Story 5.7: Chaos Engineering Support

**Priority**: P3 (Lower - Advanced testing)

**As** a platform engineer validating resilience
**I want** to inject failures into StreamFlow components
**So that** I can verify our system handles failures gracefully

**Scope**:
- Chaos engineering API endpoints
- Failure injection modes:
  - Database connection failures
  - Event stream delays/drops
  - Activity timeout simulation
  - Worker crashes
  - Network partitions
- Integration with chaos tools (Chaos Mesh, Gremlin)
- Metrics on recovery time
- Automated chaos experiments

**Benefits**:
- Validate failure handling
- Improve resilience
- Confidence in production
- Identify weaknesses before real failures

---

### Story 5.8: Activity Timeout Detection and Auto-Recovery

**Priority**: P1 (High - Production reliability)

**As** a platform engineer running StreamFlow in production
**I want** automatic detection and recovery from timed-out activities
**So that** workflows don't hang forever when workers crash or become unresponsive

**Background**:
StreamFlow currently has heartbeat endpoints (`POST /api/v1/activities/{id}/heartbeat`) but no automatic timeout detection. When a worker crashes mid-execution or becomes unresponsive, the activity remains in "running" state indefinitely, causing workflow hangs.

**Current State**:
- ✅ Heartbeat endpoint exists for workers to extend timeouts
- ❌ No background timeout detection process
- ❌ No automatic failure/retry for timed-out activities
- ✅ Timeout configuration per activity (`settings.timeout`)

**Scope**:
- Background timeout detector process
- Automatic detection of expired activities (no heartbeat within timeout window)
- Automatic failure + retry for timed-out activities
- Configurable timeout per activity (via `settings.timeout_seconds`)
- Configurable detection interval (default: 10 seconds)
- Metrics: timeout detection count, recovery success rate

**Implementation**:

**Background Detector (Rust)**:
```rust
pub async fn run_timeout_detector(
    pool: PgPool,
    config: TimeoutConfig,
    shutdown_token: CancellationToken,
) {
    let mut interval = tokio::time::interval(config.check_interval);

    loop {
        if shutdown_token.is_cancelled() {
            break;
        }

        interval.tick().await;

        match detect_and_fail_expired_activities(&pool).await {
            Ok(count) if count > 0 => {
                warn!("Detected and failed {} expired activities", count);
            }
            Err(e) => {
                error!("Timeout detection failed: {}", e);
            }
            _ => {}
        }
    }
}

async fn detect_and_fail_expired_activities(pool: &PgPool) -> Result<i64> {
    // Find activities where claimed_at + timeout < NOW() and no recent heartbeat
    // Use claimed_at as the starting point, since that's when worker claimed it
    let expired = sqlx::query!(
        r#"
        SELECT id, workflow_id, activity_key, claimed_by, timeout_seconds
        FROM activity_queue
        WHERE status = 'running'::activity_status
          AND claimed_at + make_interval(secs => timeout_seconds) < NOW()
        LIMIT 100
        "#
    )
    .fetch_all(pool)
    .await?;

    let count = expired.len() as i64;

    // For each expired activity, fail it (triggers retry logic)
    for activity in expired {
        warn!(
            activity_id = %activity.id,
            workflow_id = %activity.workflow_id,
            activity_key = %activity.activity_key,
            claimed_by = ?activity.claimed_by,
            timeout_seconds = activity.timeout_seconds,
            "Activity timed out, failing for retry"
        );

        // Call activity_queue.fail() to trigger retry logic
        // This will respect max_retries and retry_count
        let _ = sqlx::query!(
            r#"
            UPDATE activity_queue
            SET status = 'pending'::activity_status,
                claimed_by = NULL,
                claimed_at = NULL,
                scheduled_for = NOW(),
                retry_count = retry_count + 1
            WHERE id = $1
              AND retry_count < max_retries
            "#,
            activity.id
        )
        .execute(pool)
        .await;

        // If max retries exceeded, mark as failed permanently
        let _ = sqlx::query!(
            r#"
            UPDATE activity_queue
            SET status = 'failed'::activity_status,
                completed_at = NOW()
            WHERE id = $1
              AND retry_count >= max_retries
            "#,
            activity.id
        )
        .execute(pool)
        .await;

        // Publish ActivityFailed event for orchestrator
        // ... (event publishing logic)
    }

    Ok(count)
}
```

**Configuration**:
```bash
STREAMFLOW_TIMEOUT_CHECK_INTERVAL_SECONDS=10  # How often to check for timeouts
STREAMFLOW_DEFAULT_ACTIVITY_TIMEOUT_SECONDS=300  # Default timeout if not specified
```

**Activity Settings** (YAML):
```yaml
activities:
  - key: long_running_task
    worker: data
    name: process_large_dataset
    settings:
      timeout_seconds: 3600  # 1 hour timeout
      max_retries: 3         # Retry up to 3 times
```

**Heartbeat Usage** (Worker SDK):
```rust
// Worker extends timeout by calling heartbeat during long operations
for chunk in large_dataset.chunks() {
    process_chunk(chunk).await?;

    // Extend timeout (resets claimed_at timestamp)
    worker.heartbeat(activity_id).await?;
}
```

**Benefits**:
- ✅ Automatic recovery from worker crashes
- ✅ Workflows don't hang indefinitely
- ✅ No manual intervention needed
- ✅ Respects retry logic (max_retries, exponential backoff)
- ✅ Configurable timeouts per activity
- ✅ Metrics for monitoring timeout frequency

**Trade-offs**:
- Background process adds overhead (minimal - runs every 10s)
- False positives possible if detection interval > timeout (mitigated by heartbeats)
- Workers must implement heartbeat for long-running activities

**Testing**:
- Unit test: Timeout detection logic
- Integration test: Simulate worker crash mid-execution
- Integration test: Verify heartbeat extends timeout
- Load test: Verify detector handles high activity volume
- Chaos test: Kill workers randomly, verify auto-recovery

**Success Criteria**:
- ✅ Activities time out within 2x detection interval of actual timeout
- ✅ Timed-out activities retry automatically (respecting max_retries)
- ✅ Heartbeats successfully extend timeouts
- ✅ Metrics show timeout detection and recovery rates
- ✅ No false positives under normal operation

**Architecture Reference**:
- `docs/2025-11-14-analysis-absurd-orchestrator.md` Section 8 (Timeout Detection with Heartbeats)

---

## Epic 6: Advanced Workflow Features

**Goal**: Enable sophisticated workflow patterns beyond basic sequential/parallel execution.

### Story 6.1: Workflow Versioning

**Priority**: P1 (High - Production requirement)

**As** a workflow developer
**I want** to deploy new versions of workflows without affecting running instances
**So that** I can iterate safely without disrupting production

**Scope**:
- Version field in workflow definitions (semantic versioning)
- Running workflows pin to deployed version
- New workflows use latest version by default
- Version selection at workflow start (optional)
- Workflow migration tools (upgrade running workflows)
- Rollback support (deploy previous version)
- Version comparison UI (diff views)

**API**:
```bash
# Deploy new version
POST /api/v1/workflows/definitions
{"name": "payment_processing", "version": "2.0.0", "definition": {...}}

# Start workflow with specific version
POST /api/v1/workflows
{"workflow_type": "payment_processing", "version": "1.5.0", "args": {...}}

# Start workflow with latest
POST /api/v1/workflows
{"workflow_type": "payment_processing", "args": {...}}
```

**Benefits**:
- Safe deployments (no impact on running workflows)
- A/B testing (run two versions side-by-side)
- Gradual rollout (canary deployments)
- Quick rollback on issues

---

### Story 6.2: Subworkflows

**Priority**: P2 (Medium - Reusability)

**As** a workflow developer
**I want** to call one workflow from another
**So that** I can reuse common workflow patterns

**Scope**:
- Subworkflow activity type (special activity)
- Pass parameters to subworkflow
- Return outputs from subworkflow
- Nested error handling (subworkflow failure)
- Subworkflow versioning (pin version or use latest)
- Visualization (show hierarchy)
- Limits on nesting depth (prevent infinite recursion)

**Example**:
```yaml
activities:
  - key: process_order
    worker: workflows
    name: subworkflow
    parameters:
      workflow_type: process_payment
      version: "1.0"
      args:
        card_token: "{{ARG.card_token}}"
        amount: "{{ARG.amount}}"
    outputs:
      - payment_id  # From subworkflow
```

**Benefits**:
- Reusable workflow components
- Simpler workflow definitions
- Modular architecture
- Better organization

---

### Story 6.3: Dynamic Parallelism (Map/Reduce)

**Priority**: P2 (Medium - Common pattern)

**As** a workflow developer
**I want** to execute an activity N times in parallel (determined at runtime)
**So that** I can process lists/batches efficiently

**Scope**:
- Map activity type (fan-out over list)
- Reduce activity type (aggregate results)
- Dynamic list from previous activity output
- Concurrency limits (max parallel executions)
- Partial failure handling (continue on errors)
- Progress tracking (M of N complete)

**Example**:
```yaml
activities:
  - key: fetch_users
    worker: data
    name: query_users
    outputs:
      - user_ids  # Returns ["user1", "user2", "user3", ...]

  - key: process_users
    type: map  # Special activity type
    over: "{{fetch_users.user_ids}}"
    activity:
      worker: users
      name: send_email
      parameters:
        user_id: "{{item}}"
    max_concurrency: 10

  - key: aggregate_results
    type: reduce
    from: process_users
    activity:
      worker: reporting
      name: generate_summary
```

**Benefits**:
- Process large datasets efficiently
- Dynamic workflows (list size unknown upfront)
- Common ETL pattern
- Better performance (parallel processing)

---

### Story 6.4: Workflow Pause/Resume

**Priority**: P2 (Medium - Manual intervention)

**As** a workflow operator
**I want** to pause a running workflow and resume it later
**So that** I can handle manual approval steps or external dependencies

**Scope**:
- Pause workflow API (stop scheduling new activities)
- Resume workflow API (continue from paused state)
- Wait-for-event activity (external signal)
- Manual approval step (human-in-the-loop)
- Timeout on pause (auto-resume or fail)
- UI for paused workflows

**API**:
```bash
POST /api/v1/workflows/{id}/pause
POST /api/v1/workflows/{id}/resume
POST /api/v1/workflows/{id}/signal
{"event": "approval_granted", "data": {...}}
```

**Use Cases**:
- Manual approval steps
- Wait for external webhook
- Debugging (pause to inspect state)
- Rate limiting (pause until quota resets)

**Benefits**:
- Human-in-the-loop workflows
- External system integration
- Better control over execution

---

### Story 6.5: Scheduled/Cron Workflows

**Priority**: P2 (Medium - Common requirement)

**As** a workflow developer
**I want** to run workflows on a schedule (cron-like)
**So that** I can implement batch jobs and recurring tasks

**Scope**:
- Cron expression support (standard cron syntax)
- Timezone handling
- Missed execution policy (catch up, skip)
- Concurrency control (skip if previous still running)
- Next execution time calculation
- Schedule management API
- Dashboard for scheduled workflows

**Example**:
```yaml
workflow: daily_report
schedule:
  cron: "0 9 * * *"  # 9 AM daily
  timezone: "America/New_York"
  concurrency: skip  # Don't start if previous still running
```

**Benefits**:
- Replace traditional cron jobs
- Centralized scheduling
- Better monitoring and error handling
- Dependency management (workflow vs shell scripts)

---

### Story 6.6: Workflow Cancellation

**Priority**: P2 (Medium - Operational control)

**As** a workflow operator
**I want** to cancel a running workflow
**So that** I can stop workflows that are stuck or no longer needed

**Background**:
Currently, workflows run to completion or fail. There's no way to abort a running workflow, which is problematic for:
- Duplicate submissions (user retries)
- Runaway workflows (infinite loops)
- Cost control (expensive LLM workflows)
- Testing/development (quick cleanup)

**Scope**:
- Cancel workflow API endpoint
- Graceful cancellation (let running activities finish)
- Forced cancellation (immediate termination)
- Cleanup of resources (artifacts, pending activities)
- Cancellation events (publish to workflow_events)
- Terminal status: "cancelled"
- Reason tracking (audit trail)
- No explicit cancellation checks in activities (use heartbeat mechanism)

**API**:

**Endpoint**: `POST /api/v1/workflows/{id}/cancel`

**Request**:
```json
{
  "reason": "Duplicate submission detected",
  "force": false  // true = immediate, false = graceful
}
```

**Response**:
```json
{
  "workflow_id": "abc-123",
  "status": "cancelled",
  "cancelled_at": "2025-11-15T10:30:00Z",
  "reason": "Duplicate submission detected",
  "activities_terminated": 3,
  "activities_completed": 2
}
```

**Implementation**:

**Database Changes**:
```sql
-- Add cancelled_at timestamp
ALTER TABLE workflows ADD COLUMN cancelled_at TIMESTAMPTZ;
ALTER TABLE workflows ADD COLUMN cancellation_reason TEXT;

-- Add cancelled status to workflow_status enum
ALTER TYPE workflow_status ADD VALUE IF NOT EXISTS 'cancelled';
```

**Cancellation Logic** (Rust):
```rust
pub async fn cancel_workflow(
    pool: &PgPool,
    workflow_id: Uuid,
    reason: String,
    force: bool,
) -> Result<CancellationResult> {
    let mut tx = pool.begin().await?;

    // Update workflow status
    sqlx::query!(
        r#"
        UPDATE workflows
        SET status = 'cancelled'::workflow_status,
            cancelled_at = NOW(),
            cancellation_reason = $2,
            updated_at = NOW()
        WHERE id = $1 AND status NOT IN ('completed'::workflow_status, 'failed'::workflow_status)
        "#,
        workflow_id,
        reason
    )
    .execute(&mut *tx)
    .await?;

    if force {
        // Forced cancellation: Remove all pending activities immediately
        let terminated = sqlx::query!(
            r#"
            DELETE FROM activity_queue
            WHERE workflow_id = $1 AND status = 'pending'::activity_status
            "#,
            workflow_id
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();

        // Mark running activities as cancelled (they'll be cleaned up when heartbeat fails)
        sqlx::query!(
            r#"
            UPDATE activity_queue
            SET status = 'failed'::activity_status,
                completed_at = NOW()
            WHERE workflow_id = $1 AND status = 'running'::activity_status
            "#,
            workflow_id
        )
        .execute(&mut *tx)
        .await?;
    } else {
        // Graceful cancellation: Only remove pending activities
        let terminated = sqlx::query!(
            r#"
            DELETE FROM activity_queue
            WHERE workflow_id = $1 AND status = 'pending'::activity_status
            "#,
            workflow_id
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();

        // Running activities continue until completion
        // (no explicit cancellation checks needed)
    }

    // Publish WorkflowCancelled event
    sqlx::query!(
        r#"
        INSERT INTO workflow_events (id, workflow_id, event_type, payload, timestamp)
        VALUES ($1, $2, $3, $4, NOW())
        "#,
        Uuid::now_v7(),
        workflow_id,
        "WorkflowCancelled" as WorkflowEventType,
        json!({"reason": reason, "force": force})
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(CancellationResult { ... })
}
```

**Cancellation Detection** (Long-Running Activities):

StreamFlow uses **heartbeat mechanism** instead of explicit cancellation checks:

```rust
// Worker SDK (simplified - NO explicit cancellation checks needed)
impl Worker {
    pub async fn execute_long_activity(&self, activity: Activity) -> Result<Output> {
        // For long-running activities, send heartbeats
        let heartbeat_interval = Duration::from_secs(30);
        let mut heartbeat_ticker = tokio::time::interval(heartbeat_interval);

        loop {
            tokio::select! {
                // Process work
                result = process_chunk() => {
                    if result.is_done() {
                        return Ok(result.output);
                    }
                }

                // Send heartbeat periodically
                _ = heartbeat_ticker.tick() => {
                    // Heartbeat endpoint will return 404 or error if workflow cancelled
                    match self.heartbeat(activity.id).await {
                        Ok(_) => continue,  // Workflow still active
                        Err(HeartbeatError::WorkflowCancelled) => {
                            return Err(Error::Cancelled);  // Graceful exit
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }
    }
}
```

**Heartbeat Enhancement** (Detects Cancellation):
```rust
// Update heartbeat endpoint to check workflow cancellation
pub async fn heartbeat_activity(
    pool: &PgPool,
    activity_id: Uuid,
    worker_id: &str,
) -> Result<HeartbeatResponse> {
    // Check if workflow was cancelled
    let workflow_status = sqlx::query_scalar!(
        r#"
        SELECT w.status AS "status: WorkflowStatus"
        FROM workflows w
        JOIN activity_queue a ON w.id = a.workflow_id
        WHERE a.id = $1
        "#,
        activity_id
    )
    .fetch_one(pool)
    .await?;

    if workflow_status == WorkflowStatus::Cancelled {
        return Err(HeartbeatError::WorkflowCancelled);
    }

    // Normal heartbeat logic
    // ...
}
```

**Benefits**:
- ✅ Clean up stuck workflows (infinite loops, hung workers)
- ✅ Stop expensive workflows (cost control for LLM workflows)
- ✅ Handle duplicate submissions gracefully
- ✅ Better operational control (UI cancel buttons)
- ✅ **No explicit cancellation checks** - uses existing heartbeat mechanism
- ✅ Short-running activities don't need changes (complete before cancellation matters)
- ✅ Long-running activities detect cancellation via heartbeat errors

**Design Decision - Why Not Explicit Cancellation Checks**:

StreamFlow uses heartbeat mechanism instead of absurd's `ctx.isCancelled()` checks:

1. **Heartbeats already exist** - Long-running activities already send heartbeats
2. **Simpler worker SDKs** - No need for cancellation context passing
3. **Sufficient for use case** - Cancellation within heartbeat interval (30s) is acceptable
4. **Short activities don't care** - Activities that run <30s complete before cancellation matters
5. **Graceful exit** - Heartbeat failure triggers graceful activity exit

**Trade-offs**:
- Cancellation latency = heartbeat interval (30 seconds typical)
- Short-running activities may complete before cancellation takes effect
- Acceptable trade-off for simpler implementation

**Testing**:
- Unit test: Graceful vs forced cancellation logic
- Integration test: Cancel workflow with pending activities
- Integration test: Cancel workflow with running activities
- Integration test: Heartbeat fails when workflow cancelled
- E2E test: UI cancel button workflow

**Success Criteria**:
- ✅ Cancelled workflows transition to "cancelled" status
- ✅ Pending activities removed from queue
- ✅ Running activities detect cancellation within 2x heartbeat interval
- ✅ Graceful cancellation completes running activities
- ✅ Forced cancellation terminates immediately
- ✅ WorkflowCancelled event published

**Architecture Reference**:
- `docs/2025-11-14-analysis-absurd-orchestrator.md` Section 11 (Cancellation Support)

---

### Story 6.7: Workflow Retry Policies

**Priority**: P2 (Medium - Reliability)

**As** a workflow developer
**I want** to configure retry behavior for entire workflows
**So that** transient failures don't require manual intervention

**Scope**:
- Workflow-level retry configuration
- Max retries per workflow
- Backoff strategy (exponential, linear)
- Retry conditions (which error types to retry)
- Retry from specific activity (partial retry)
- Retry history tracking

**Configuration**:
```yaml
workflow: flaky_integration
retry:
  max_attempts: 3
  backoff:
    type: exponential
    initial: 60s
    max: 3600s
  retry_on:
    - "TimeoutError"
    - "NetworkError"
  retry_from: last_failed_activity
```

**Benefits**:
- Automatic recovery from transient failures
- Reduced operational overhead
- Better reliability
- Configurable behavior per workflow

---

### Story 6.8: Workflow Definition Validation with Cycle Detection

**Priority**: P2 (Medium - Developer experience and safety)

**As** a workflow developer
**I want** workflow definitions to be validated at deployment time
**So that** I catch errors before workflows execute in production

**Scope**:
- Validate workflow definition structure (YAML/JSON schema)
- Validate activity references (all `depends_on`/`dependency_of` keys exist)
- **Cycle detection in workflow graph**
- **Ensure all cycles are conditional** (unconditional cycles rejected)
- Validate parameter templates (proper syntax)
- Validate condition expressions (parseable)
- Clear error messages with line numbers
- Validation API endpoint (for CI/CD integration)

**Cycle Detection Requirements**:
- Detect cycles in the directed graph (activity A → B → C → A)
- Allow conditional cycles (loops with conditions on edges)
- Reject unconditional cycles (infinite loops without escape conditions)
- Error message shows cycle path: `validate → process → retry → validate`

**Example Valid Conditional Cycle**:
```yaml
activities:
  - key: validate_input
    worker: validation
    name: check_data
    dependency_of:
      - activity_key: process_data
        conditions:
          - "{{validate_input.valid}} == true"
      - activity_key: retry_validation
        conditions:
          - "{{validate_input.valid}} == false AND {{validate_input.retry_count}} < 3"

  - key: process_data
    worker: processing
    name: transform

  - key: retry_validation
    worker: validation
    name: check_data_retry
    dependency_of:
      - activity_key: validate_input  # Cycle back, but conditional
```

**Example Invalid Unconditional Cycle**:
```yaml
activities:
  - key: step_a
    worker: test
    name: do_something
    dependency_of:
      - activity_key: step_b

  - key: step_b
    worker: test
    name: do_something_else
    dependency_of:
      - activity_key: step_a  # ERROR: Unconditional cycle detected!
```

**Implementation Notes**:
- For MVP: Simple cycle detection using depth-first search (DFS)
- Check if all edges in a cycle have conditions
- Graph library (petgraph) could be used but not required for ~10 activities
- Post-MVP: More sophisticated analysis (reachability, dead code detection)

**Benefits**:
- Catch errors at deployment time (not runtime)
- Prevent infinite loops in workflows
- Better developer experience (clear error messages)
- Safe conditional loops (retry logic, polling)
- Confidence in workflow correctness

---

## Epic 7: Scalability Enhancements

**Goal**: Scale StreamFlow to handle millions of workflows per day with minimal infrastructure.

### Story 7.1: Read Replicas for Queries

**Priority**: P1 (High - Offload primary database)

**As** a platform engineer scaling StreamFlow
**I want** to route read queries to PostgreSQL replicas
**So that** the primary database handles only writes

**Scope**:
- Separate connection pools (primary, replicas)
- Read/write splitting in query layer
- Replica lag monitoring
- Fallback to primary if replicas unavailable
- Configuration for replica endpoints

**Routing Rules**:
- Writes → Primary
- Event polling → Replicas (eventual consistency ok)
- Workflow status queries → Replicas
- Activity queue claims → Primary (requires consistency)

**Benefits**:
- Reduced load on primary database
- Higher read throughput
- Better write performance (fewer read locks)

---

### Story 7.2: Connection Pooling (PgBouncer)

**Priority**: P2 (Medium - Reduce connection overhead)

**As** a platform engineer
**I want** to use PgBouncer for connection pooling
**So that** I can handle more concurrent requests with fewer database connections

**Scope**:
- PgBouncer deployment and configuration
- Transaction-mode pooling (most efficient)
- Connection limits and queuing
- Monitoring (pool utilization)
- Failover configuration

**Benefits**:
- 10-100x reduction in database connections
- Better resource utilization
- Faster connection acquisition
- Reduced database overhead

---

### Story 7.3: Horizontal Sharding

**Priority**: P3 (Lower - Only for extreme scale)

**As** a platform engineer scaling beyond single database capacity
**I want** to shard data across multiple PostgreSQL instances
**So that** I can scale beyond single database limits

**Scope**:
- Shard by workflow_id or tenant_id
- Shard key routing in query layer
- Shard rebalancing (add/remove shards)
- Cross-shard queries (limited support)
- Monitoring per shard

**Benefits**:
- Scale beyond single database (100k+ workflows/sec)
- Better isolation (per-tenant shards)
- Geographic distribution (latency optimization)

**Trade-offs**:
- High complexity
- Limited cross-shard operations
- Operational overhead

---

### Story 7.4: Event Stream Batching

**Priority**: P2 (Medium - Throughput optimization)

**As** an orchestrator
**I want** to process multiple events in a single batch
**So that** I reduce database roundtrips and improve throughput

**Scope**:
- Batch event polling (return up to 100 events)
- Batch activity scheduling (insert multiple activities at once)
- Batch event publishing (insert multiple events at once)
- Batch size tuning (performance vs latency)
- Metrics on batch sizes

**Benefits**:
- Reduced database roundtrips
- Higher throughput
- Better resource utilization
- Lower latency per event (amortized overhead)

---

### Story 7.5: Parallel Workflow Event Processing (Per-Workflow Task Spawning)

**Priority**: P2 (Medium - Significant performance improvement with acceptable complexity)

**As** an orchestrator
**I want** to process events for different workflows concurrently
**So that** I maximize CPU utilization and achieve 10-100x throughput improvement

**Scope**:
- Group events by workflow_id after polling
- Spawn one Tokio task per workflow (not per event)
- Process events for same workflow sequentially within task (maintains ordering)
- Configurable concurrency limit (semaphore-based backpressure)
- Per-workflow advisory locking (prevents concurrent evaluation of same workflow)
- **Per-event checkpointing**: Each event checkpointed immediately after processing (no replay on shutdown)
- Graceful error handling (one workflow failure doesn't stop others)
- Enhanced observability (metrics, tracing, health checks)

**Architecture Reference**: See detailed implementation plan in `docs/implementation/US-7.5-parallel-workflow-event-processing.md`

**Performance Targets**:
- **Multi-workflow throughput**: 10,000-100,000 workflows/sec (10-100x improvement)
- **Latency P95**: <5ms (5x improvement from ~10ms)
- **CPU utilization**: 60-80% (improved from ~10% due to I/O bound operations)
- **Connection pool**: Scale to 2x concurrency (200 connections for concurrency=100)

**Benefits**:
- ✅ 10-100x better multi-workflow throughput (many independent workflows)
- ✅ 5-10x better latency distribution (no head-of-line blocking)
- ✅ Better CPU utilization (parallel processing during I/O waits)
- ✅ Maintains correctness (advisory locks serialize same-workflow events)
- ✅ Maintains at-least-once semantics (checkpoints after task completion)
- ✅ Graceful degradation (configurable concurrency, semaphore backpressure)

**Trade-offs**:
- ❌ Increased complexity (~400 LOC vs ~200 LOC for sequential)
- ❌ Higher connection pool requirements (2x concurrency + overhead)
- ❌ Longer graceful shutdown (wait for all in-flight workflows)
- ⚠️ No benefit for single-workflow throughput (advisory locks still serialize)

**Key Design Decisions**:
1. **Per-workflow grouping**: Events grouped by workflow_id after polling
2. **Sequential within workflow**: Each task processes its workflow's events in order
3. **Concurrent across workflows**: Different workflows process in parallel (10-100 concurrent)
4. **Advisory lock coordination**: PostgreSQL serializes same-workflow events automatically
5. **Per-event checkpointing**: Each event checkpointed immediately (no replay on shutdown)
6. **Concurrency semantics**: 0=disabled, 1=sequential (MVP), N=parallel

**Example Scenario**:
```
Polled batch: 100 events across 20 workflows
├── Spawn 20 tasks (one per workflow, limited by semaphore to max_concurrent)
├── Workflow A task: Process 10 events sequentially
│   └── Each event checkpointed immediately after processing (advisory lock prevents conflicts)
├── Workflow B task: Process 5 events sequentially (runs concurrently with Workflow A)
│   └── Each event checkpointed immediately after processing
└── ... (18 more workflows processing in parallel, each checkpointing independently)
└── Await all tasks (collect statistics, no centralized checkpointing needed)
```

**Configuration**:
```bash
# Orchestrator concurrency
# 0 = disabled (maintenance mode - no workflows process)
# 1 = sequential (current MVP behavior - one workflow at a time)
# N = parallel (N workflows process concurrently)
STREAMFLOW_ORCHESTRATOR_MAX_CONCURRENT=100

# Events to poll per batch (larger = better throughput)
STREAMFLOW_ORCHESTRATOR_POLL_BATCH_SIZE=100

# Database connection pool must scale (2x concurrency + 40 for API/workers)
# Example: 100 concurrent = 240 connections
```

**Rollout Plan**:
- **Phase 1**: Deploy with max_concurrent=1 (sequential mode - current MVP behavior)
- **Phase 2**: Gradual increase (1 → 10 → 50 → 100) with monitoring at each step
- **Phase 3**: Production deployment with max_concurrent=100 as default
- **Rollback**: Set `STREAMFLOW_ORCHESTRATOR_MAX_CONCURRENT=1` to revert to sequential

**Success Metrics**:
- ✅ Throughput: >10,000 workflows/sec (10x improvement)
- ✅ Latency P95: <5ms (5x improvement)
- ✅ Error rate: <0.1% (no regression)
- ✅ Connection pool: <90% utilization (no exhaustion)

**Implementation Estimate**: 2-3 weeks
- Week 1: Core parallel processing logic
- Week 2: Configuration, resource management, observability
- Week 3: Testing, load testing, documentation

**When to Use**:
- ✅ Multi-tenant SaaS (many independent workflows)
- ✅ Batch processing (thousands of workflows)
- ✅ High-throughput requirements (>100 workflows/sec)
- ✅ Real-time applications (P95 latency matters)
- ❌ Single-tenant with few workflows (<10 concurrent)
- ❌ Limited database connections (<100 available)
- ❌ Simplicity preferred over performance

---

## Prioritization Framework

Post-MVP features are prioritized using:

1. **Customer Impact**: How many customers benefit?
2. **Architectural Dependency**: Does it block other features?
3. **Competitive Advantage**: Does it differentiate StreamFlow?
4. **Effort**: Engineering time required (story points)

**Priority Levels**:
- **P0 (Critical)**: Must have for MVP (none in this doc - all deferred)
- **P1 (High)**: Should have in first post-MVP release
- **P2 (Medium)**: Nice to have in early releases
- **P3 (Lower)**: Future consideration, niche use cases

---

## Phased Rollout Recommendation

### Phase 1 (Post-MVP Release 0.3) - 3 months
**Focus**: External integrations and basic enterprise features

**Stories**:
- Epic 1: Auth0/Okta integration (1.1)
- Epic 1: Kafka event streaming (1.2)
- Epic 1: S3 artifact storage (1.8)
- Epic 2: Compiled workflows (2.1)
- Epic 2: Workflow state caching (2.2)
- **Epic 2: Event table partitioning (2.3)** - Critical for audit compliance at scale
- Epic 4: Python SDK (4.1)
- Epic 5: Metrics and monitoring (5.1)
- Epic 5: High availability setup (5.4)
- **Epic 5: Activity timeout detection (5.8)** - Prevent hung workflows from worker crashes
- Epic 6: Workflow versioning (6.1)
- Epic 7: Read replicas (7.1)

**Value**: Production-ready for high-scale deployments with external auth, high throughput, scalable audit retention, and automatic timeout recovery.

---

### Phase 2 (Release 0.4) - 3 months
**Focus**: Multi-tenancy and developer experience

**Stories**:
- Epic 3: Row-level security (3.1)
- Epic 3: RBAC (3.2)
- Epic 3: Per-tenant quotas (3.3)
- Epic 4: TypeScript SDK (4.2)
- Epic 4: Complex expressions (4.4)
- Epic 4: CLI tool (4.6)
- Epic 5: Structured logging (5.3)
- Epic 5: Distributed tracing (5.2)
- Epic 6: Subworkflows (6.2)
- Epic 6: Workflow pause/resume (6.4)
- **Epic 6: Workflow cancellation (6.6)** - Operational control via heartbeat mechanism

**Value**: SaaS-ready with multi-tenancy, better DX tools, and advanced workflow features including operational controls (pause/resume/cancel).

---

### Phase 3 (Release 0.5) - 3 months
**Focus**: Advanced features and scalability

**Stories**:
- Epic 1: NATS JetStream (1.3)
- Epic 1: RabbitMQ queue (1.6)
- Epic 1: Redis caching (1.10)
- Epic 2: Event table partitioning (2.3)
- Epic 2: Priority queues (2.5)
- Epic 4: Web workflow designer (4.5)
- Epic 4: Activity development kit (4.7)
- Epic 6: Dynamic parallelism (6.3)
- Epic 6: Scheduled workflows (6.5)
- Epic 7: Event stream batching (7.4)

**Value**: Enterprise-scale with advanced features, visual designer, and ultra-high throughput.

---

### Phase 4+ (Release 0.6+) - Ongoing
**Focus**: Niche features and optimizations

**Stories**:
- Remaining P2 and P3 stories
- Customer-specific requests
- Performance optimizations based on production usage
- New features based on market feedback

---

## Success Metrics

Track these metrics to measure post-MVP progress:

**Adoption**:
- Number of production deployments
- Number of workflows executed per day
- Number of custom activities developed
- Community contributions (PRs, issues)

**Performance**:
- P95 workflow latency
- Workflows per second (sustained)
- Database query performance (P95)
- Resource utilization (CPU, RAM, disk)

**Reliability**:
- Uptime (99.9%+ target)
- Error rates (by error type)
- Mean time to recovery (MTTR)
- Data loss incidents (target: zero)

**Developer Experience**:
- Time to first workflow (onboarding)
- SDK adoption rates
- Documentation usage
- Support ticket volume

---

## References

- MVP Architecture: `docs/architecture.md`
- US-1.1 Implementation: `docs/implementation/US-1.1-activity-queue.md`
- US-1.2 Implementation: `docs/implementation/US-1.2-event-driven-scheduling.md`

---

## Contributing

This roadmap is a living document. As we learn from MVP deployments and customer feedback, priorities may shift. Suggestions for new features or changes to priorities are welcome via GitHub issues.

**Process**:
1. Open issue with `[Post-MVP]` tag
2. Describe use case and value proposition
3. Community discussion and prioritization
4. If accepted, add to this document with epic/story structure
5. Assign to release phase based on priority

---

**Last Updated**: 2025-10-29
**Next Review**: After MVP release
