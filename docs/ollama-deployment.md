# Ollama Deployment Guide

This guide covers deploying and configuring Ollama for use with Kruxia Flow LLM activities.

## Overview

Ollama is a self-hosted LLM provider that enables running models locally or in your infrastructure. Kruxia Flow supports Ollama as a first-class LLM provider alongside cloud providers like Anthropic, OpenAI, and Google.

## Configuration

Kruxia Flow workers discover and connect to Ollama via environment variables:

| Variable          | Required | Default               | Description                                    |
|-------------------|----------|-----------------------|------------------------------------------------|
| `OLLAMA_BASE_URL` | No       | `http://localhost:11434` | Ollama API endpoint URL                     |
| `OLLAMA_API_KEY`  | No       | None                  | Optional API key for secured Ollama instances |

## Deployment Scenarios

### 1. Local Development

For local development, install and run Ollama on your development machine.

**Install Ollama**:
```bash
# macOS
brew install ollama

# Linux
curl -fsSL https://ollama.com/install.sh | sh
```

**Start Ollama**:
```bash
ollama serve
```

**Pull models**:
```bash
# Pull models you want to use
ollama pull llama3.2
ollama pull mistral
ollama pull codellama
```

**Configure Kruxia Flow worker**:
```bash
# Default configuration (localhost:11434)
cargo run -p kruxiaflow-worker

# Or explicitly set the URL
OLLAMA_BASE_URL=http://localhost:11434 cargo run -p kruxiaflow-worker
```

**Workflow example**:
```yaml
activities:
  - key: analyze_code
    worker: std
    activity_name: llm_prompt
    parameters:
      model: ollama/llama3.2
      prompt: "Review this code for security issues: {{INPUT.code}}"
      max_tokens: 500
```

---

### 2. Docker Deployment

Run Ollama in a Docker container alongside Kruxia Flow services.

**Docker Compose example**:
```yaml
version: '3.8'

services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: kruxiaflow
      POSTGRES_USER: kruxiaflow
      POSTGRES_PASSWORD: kruxiaflow_dev
    ports:
      - "5432:5432"
    volumes:
      - postgres_data:/var/lib/postgresql/data

  ollama:
    image: ollama/ollama:latest
    ports:
      - "11434:11434"
    volumes:
      - ollama_models:/root/.ollama
    # Optional: GPU support (NVIDIA)
    # deploy:
    #   resources:
    #     reservations:
    #       devices:
    #         - driver: nvidia
    #           count: 1
    #           capabilities: [gpu]

  kruxiaflow-api:
    image: kruxiaflow:latest
    command: serve
    environment:
      KRUXIAFLOW_DATABASE_URL: postgres://kruxiaflow:kruxiaflow_dev@postgres:5432/kruxiaflow
      KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM: ${KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM}
    ports:
      - "8080:8080"
    depends_on:
      - postgres

  kruxiaflow-worker:
    image: kruxiaflow:latest
    command: worker
    environment:
      KRUXIAFLOW_DATABASE_URL: postgres://kruxiaflow:kruxiaflow_dev@postgres:5432/kruxiaflow
      OLLAMA_BASE_URL: http://ollama:11434
      # Cloud provider API keys (optional)
      # ANTHROPIC_API_KEY: ${ANTHROPIC_API_KEY}
      # OPENAI_API_KEY: ${OPENAI_API_KEY}
      # GOOGLE_API_KEY: ${GOOGLE_API_KEY}
    depends_on:
      - postgres
      - ollama

volumes:
  postgres_data:
  ollama_models:
```

**Pull models into Docker container**:
```bash
# After starting the stack
docker exec -it <ollama-container-id> ollama pull llama3.2
docker exec -it <ollama-container-id> ollama pull mistral

# Or use a helper script
docker-compose exec ollama ollama pull llama3.2
```

**Verify connection**:
```bash
# From worker container
docker-compose exec kruxiaflow-worker curl http://ollama:11434/api/tags
```

---

### 3. Kubernetes Deployment

Deploy Ollama as a Kubernetes deployment with persistent storage for models.

**Ollama Deployment**:
```yaml
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: ollama-models-pvc
  namespace: kruxiaflow
spec:
  accessModes:
    - ReadWriteOnce
  resources:
    requests:
      storage: 50Gi  # Adjust based on model sizes
  storageClassName: standard  # Use your storage class

---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ollama
  namespace: kruxiaflow
spec:
  replicas: 1
  selector:
    matchLabels:
      app: ollama
  template:
    metadata:
      labels:
        app: ollama
    spec:
      containers:
      - name: ollama
        image: ollama/ollama:latest
        ports:
        - containerPort: 11434
          name: http
        volumeMounts:
        - name: models
          mountPath: /root/.ollama
        resources:
          requests:
            memory: "4Gi"
            cpu: "2"
          limits:
            memory: "8Gi"
            cpu: "4"
            # GPU support (NVIDIA)
            # nvidia.com/gpu: 1
      volumes:
      - name: models
        persistentVolumeClaim:
          claimName: ollama-models-pvc

---
apiVersion: v1
kind: Service
metadata:
  name: ollama
  namespace: kruxiaflow
spec:
  selector:
    app: ollama
  ports:
  - port: 11434
    targetPort: 11434
    name: http
  type: ClusterIP
```

**Kruxia Flow Worker Deployment**:
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: kruxiaflow-worker
  namespace: kruxiaflow
spec:
  replicas: 3
  selector:
    matchLabels:
      app: kruxiaflow-worker
  template:
    metadata:
      labels:
        app: kruxiaflow-worker
    spec:
      containers:
      - name: worker
        image: kruxiaflow:latest
        command: ["kruxiaflow", "worker"]
        env:
        - name: KRUXIAFLOW_DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: kruxiaflow-secrets
              key: database-url
        - name: OLLAMA_BASE_URL
          value: "http://ollama:11434"
        # Cloud provider API keys from secrets
        - name: ANTHROPIC_API_KEY
          valueFrom:
            secretKeyRef:
              name: kruxiaflow-secrets
              key: anthropic-api-key
              optional: true
        - name: OPENAI_API_KEY
          valueFrom:
            secretKeyRef:
              name: kruxiaflow-secrets
              key: openai-api-key
              optional: true
        - name: GOOGLE_API_KEY
          valueFrom:
            secretKeyRef:
              name: kruxiaflow-secrets
              key: google-api-key
              optional: true
        resources:
          requests:
            memory: "512Mi"
            cpu: "500m"
          limits:
            memory: "1Gi"
            cpu: "1"
```

**Initialize Ollama with models (Job)**:
```yaml
apiVersion: batch/v1
kind: Job
metadata:
  name: ollama-pull-models
  namespace: kruxiaflow
spec:
  template:
    spec:
      restartPolicy: OnFailure
      containers:
      - name: pull-models
        image: curlimages/curl:latest
        command:
        - /bin/sh
        - -c
        - |
          # Wait for Ollama to be ready
          until curl -f http://ollama:11434/api/tags; do
            echo "Waiting for Ollama..."
            sleep 5
          done

          # Pull models
          curl -X POST http://ollama:11434/api/pull \
            -d '{"name": "llama3.2"}'

          curl -X POST http://ollama:11434/api/pull \
            -d '{"name": "mistral"}'

          echo "Models pulled successfully"
```

**Deploy**:
```bash
kubectl apply -f ollama-deployment.yaml
kubectl apply -f kruxiaflow-worker-deployment.yaml
kubectl apply -f ollama-init-job.yaml

# Verify Ollama is running
kubectl get pods -n kruxiaflow -l app=ollama
kubectl logs -n kruxiaflow -l app=ollama

# Check models
kubectl exec -n kruxiaflow deployment/ollama -- ollama list
```

---

### 4. Remote Ollama Instance

Connect to an Ollama instance running on a different machine.

**Configuration**:
```bash
# Point worker to remote Ollama
OLLAMA_BASE_URL=https://ollama.example.com:11434 cargo run -p kruxiaflow-worker

# With API key authentication
OLLAMA_BASE_URL=https://ollama.example.com:11434 \
OLLAMA_API_KEY=your-secret-key \
cargo run -p kruxiaflow-worker
```

**Note**: Ensure the remote Ollama instance is accessible from your worker nodes and properly secured (HTTPS, authentication).

---

## Model Management

### Available Models

List models available in your Ollama instance:

```bash
# Local
ollama list

# Docker
docker-compose exec ollama ollama list

# Kubernetes
kubectl exec -n kruxiaflow deployment/ollama -- ollama list
```

### Model Selection in Workflows

Specify Ollama models using the `ollama/` prefix:

```yaml
# Single model
parameters:
  model: ollama/llama3.2

# Fallback chain (try Ollama first, then cloud)
parameters:
  model:
    - ollama/llama3.2
    - ollama/mistral
    - anthropic/claude-3-5-haiku-20241022
```

### Model Discovery

Kruxia Flow automatically validates models against Ollama's available models with a 5-minute cache. If you specify a model that doesn't exist, you'll get a clear error:

```
Error: Model 'invalid-model' not found in Ollama. Available models: llama3.2, mistral, codellama
```

---

## GPU Support

### Docker with NVIDIA GPU

```yaml
services:
  ollama:
    image: ollama/ollama:latest
    deploy:
      resources:
        reservations:
          devices:
            - driver: nvidia
              count: 1
              capabilities: [gpu]
```

**Prerequisites**:
- NVIDIA Container Toolkit installed
- Docker configured for GPU support

### Kubernetes with GPU

```yaml
spec:
  containers:
  - name: ollama
    resources:
      limits:
        nvidia.com/gpu: 1
```

**Prerequisites**:
- NVIDIA device plugin deployed
- GPU nodes labeled and available

---

## Performance Considerations

### Model Size vs Performance

| Model Size | RAM Required | GPU VRAM | Inference Speed | Use Case                |
|------------|--------------|----------|-----------------|-------------------------|
| 7B         | 8 GB         | 4 GB     | Fast            | Development, testing    |
| 13B        | 16 GB        | 8 GB     | Medium          | Production, quality     |
| 70B+       | 64 GB+       | 40 GB+   | Slow            | High-accuracy tasks     |

### Scaling Ollama

**Vertical scaling** (recommended):
- Run Ollama on larger instances with more RAM/GPU
- Single Ollama instance can serve multiple workers
- Models stay in memory for fast inference

**Horizontal scaling**:
- Deploy multiple Ollama replicas with different models
- Use service mesh or load balancer for routing
- Each worker can target specific Ollama instances

### Resource Limits

Set appropriate resource limits based on models:

```yaml
# Docker Compose
services:
  ollama:
    deploy:
      resources:
        limits:
          memory: 16G
          cpus: '4'

# Kubernetes
resources:
  limits:
    memory: "16Gi"
    cpu: "4"
```

---

## Troubleshooting

### Connection Errors

**Symptom**: `Failed to connect to Ollama at http://localhost:11434`

**Solutions**:
1. Verify Ollama is running: `curl http://localhost:11434/api/tags`
2. Check `OLLAMA_BASE_URL` environment variable
3. Verify network connectivity (Docker network, Kubernetes service)
4. Check firewall rules

### Model Not Found

**Symptom**: `Model 'llama3.2' not found in Ollama`

**Solutions**:
1. Pull the model: `ollama pull llama3.2`
2. List available models: `ollama list`
3. Wait for model download to complete
4. Check model cache expiry (5-minute cache)

### Out of Memory

**Symptom**: Ollama crashes or slow inference

**Solutions**:
1. Use smaller models (7B instead of 13B)
2. Increase memory limits in Docker/Kubernetes
3. Enable GPU support for larger models
4. Reduce concurrent requests

### Authentication Errors

**Symptom**: `401 Unauthorized` from Ollama

**Solutions**:
1. Set `OLLAMA_API_KEY` environment variable
2. Verify API key is correct
3. Check if Ollama instance requires authentication

---

## Security Best Practices

1. **Network isolation**: Run Ollama in private network, not exposed to internet
2. **API key authentication**: Enable API keys for Ollama instances
3. **TLS encryption**: Use HTTPS for remote Ollama connections
4. **Resource limits**: Set memory/CPU limits to prevent resource exhaustion
5. **Model validation**: Only pull trusted models from official sources

---

## Cost Considerations

### Ollama Advantages
- No per-token costs
- Predictable infrastructure costs
- Data privacy (runs locally)
- No rate limits

### Infrastructure Costs

**AWS Example** (us-east-1):
| Instance Type | vCPU | RAM  | GPU          | Hourly Cost | Monthly Cost |
|---------------|------|------|--------------|-------------|--------------|
| t3.xlarge     | 4    | 16GB | None         | $0.1664     | ~$120        |
| g4dn.xlarge   | 4    | 16GB | T4 (16GB)    | $0.526      | ~$380        |
| g4dn.2xlarge  | 8    | 32GB | T4 (16GB)    | $0.752      | ~$540        |

**Cost comparison**:
- Cloud LLM: $0.25-$15 per million tokens
- Ollama: $120-$540/month (unlimited tokens)
- Break-even: ~1-10M tokens/month depending on model

---

## Related Documentation

- [LLM Activities Guide](./llm-activities.md) - Using LLM activities in workflows
- [Budget Configuration](./budget-configuration.md) - Cost tracking and limits
- [Multi-Provider Fallback](./multi-provider-fallback.md) - Fallback chains
- [Cost Dashboard API](./cost-dashboard-api.md) - Monitoring LLM costs
