"""Content Moderation & Recommendation System - Comprehensive py-std example.

This example demonstrates the full range of py-std worker capabilities:
- Core utilities: API ingestion, validation, moderation decision logic
- NLP: Embeddings, sentiment analysis, entity extraction, toxicity detection
- Machine learning: Feature engineering, engagement prediction, quality scoring
- Data processing: DuckDB analytics, trend analysis, data warehouse export

The workflow processes user-generated content (reviews, comments, posts),
moderates for inappropriate content, predicts engagement, and generates
personalized recommendations.

Architecture:
1. Ingestion Layer: Fetch, validate, enrich content
2. NLP Analysis: Embeddings, sentiment, entities, toxicity
3. ML Predictions: Feature engineering, engagement prediction
4. Analytics: Aggregate metrics, trend analysis
5. Moderation Decision: Apply business rules, route content
"""

from kruxiaflow import ScriptActivity, Workflow

# ============================================================================
# PHASE 1: INGESTION & VALIDATION
# ============================================================================


# Step 1: Fetch content batch from API
@ScriptActivity.from_function(
    inputs={
        # Sample user-generated content
        "content_batch": [
            {
                "content_id": "POST001",
                "user_id": "U123",
                "text": "This product is absolutely amazing! Best purchase ever!",
                "platform": "review",
                "timestamp": "2025-01-20T10:00:00Z",
            },
            {
                "content_id": "POST002",
                "user_id": "U456",
                "text": "Terrible service. Never buying from here again.",
                "platform": "review",
                "timestamp": "2025-01-20T11:30:00Z",
            },
            {
                "content_id": "POST003",
                "user_id": "U789",
                "text": "You are such an idiot for recommending this garbage!",
                "platform": "comment",
                "timestamp": "2025-01-20T12:15:00Z",
            },
            {
                "content_id": "POST004",
                "user_id": "U234",
                "text": "Great experience overall. The customer support team was helpful and responsive.",
                "platform": "review",
                "timestamp": "2025-01-20T14:00:00Z",
            },
            {
                "content_id": "POST005",
                "user_id": "U567",
                "text": "This is spam. Click here to win $1000!!!",
                "platform": "comment",
                "timestamp": "2025-01-20T15:30:00Z",
            },
        ],
    },
)
async def fetch_content(content_batch):
    from datetime import datetime

    return {
        "content_batch": content_batch,
        "batch_size": len(content_batch),
        "fetched_at": datetime.utcnow().isoformat(),
    }


# Step 2: Validate JSON structure with Pydantic
@ScriptActivity.from_function(
    inputs={
        "content_batch": fetch_content["content_batch"],
    },
    depends_on=["fetch_content"],
)
async def validate_schema(content_batch):
    from typing import Literal

    from pydantic import BaseModel, Field, ValidationError

    class ContentItem(BaseModel):
        content_id: str
        user_id: str
        text: str = Field(min_length=1, max_length=5000)
        platform: Literal["review", "comment", "post"]
        timestamp: str

    validated_content = []
    validation_errors = []

    for item in content_batch:
        try:
            validated = ContentItem(**item)
            validated_content.append(validated.dict())
        except ValidationError as e:
            validation_errors.append(
                {
                    "content_id": item.get("content_id", "unknown"),
                    "errors": e.errors(),
                }
            )

    return {
        "validated_content": validated_content,
        "valid_count": len(validated_content),
        "error_count": len(validation_errors),
        "validation_errors": validation_errors,
    }


# Step 3: Enrich with metadata
@ScriptActivity.from_function(
    inputs={
        "validated_content": validate_schema["validated_content"],
    },
    depends_on=["validate_schema"],
)
async def enrich_metadata(validated_content):
    from datetime import datetime, timezone

    from dateutil import parser

    content = validated_content

    for item in content:
        # Parse timestamp
        ts = parser.isoparse(item["timestamp"])
        item["parsed_timestamp"] = ts.isoformat()

        # Calculate age in hours
        now = datetime.now(timezone.utc)
        age_hours = (now - ts).total_seconds() / 3600
        item["age_hours"] = round(age_hours, 2)

        # Add text length
        item["text_length"] = len(item["text"])
        item["word_count"] = len(item["text"].split())

    return {
        "enriched_content": content,
    }


# ============================================================================
# PHASE 2: NLP ANALYSIS
# ============================================================================


# Step 4: Count tokens for monitoring
@ScriptActivity.from_function(
    inputs={
        "enriched_content": enrich_metadata["enriched_content"],
    },
    depends_on=["enrich_metadata"],
)
async def count_tokens(enriched_content):
    import tiktoken

    content = enriched_content
    encoding = tiktoken.get_encoding("cl100k_base")

    total_tokens = 0
    for item in content:
        tokens = encoding.encode(item["text"])
        item["token_count"] = len(tokens)
        total_tokens += len(tokens)

    return {
        "content": content,
        "total_tokens": total_tokens,
    }


# Step 5: Generate embeddings for similarity search
@ScriptActivity.from_function(
    inputs={
        "content": count_tokens["content"],
    },
    depends_on=["count_tokens"],
)
async def generate_embeddings(content):
    from sentence_transformers import SentenceTransformer

    model = SentenceTransformer("all-MiniLM-L6-v2")
    texts = [item["text"] for item in content]
    embeddings = model.encode(texts, show_progress_bar=False)

    for i, item in enumerate(content):
        item["embedding"] = embeddings[i].tolist()

    return {
        "content": content,
        "embedding_dimension": embeddings.shape[1],
    }


# Step 6: Analyze sentiment
@ScriptActivity.from_function(
    inputs={
        "content": generate_embeddings["content"],
    },
    depends_on=["generate_embeddings"],
)
async def analyze_sentiment(content):
    from transformers import pipeline

    sentiment_analyzer = pipeline("sentiment-analysis")

    for item in content:
        result = sentiment_analyzer(item["text"])[0]
        item["sentiment_label"] = result["label"]
        item["sentiment_score"] = round(result["score"], 4)

    return {
        "content": content,
    }


# Step 7: Extract named entities
@ScriptActivity.from_function(
    inputs={
        "content": analyze_sentiment["content"],
    },
    depends_on=["analyze_sentiment"],
)
async def extract_entities(content):
    import spacy

    nlp = spacy.load("en_core_web_sm")

    for item in content:
        doc = nlp(item["text"])
        entities = [{"text": ent.text, "label": ent.label_} for ent in doc.ents]
        item["entities"] = entities
        item["entity_count"] = len(entities)

    return {
        "content": content,
    }


# Step 8: Detect toxicity (simplified - would use specialized model in production)
@ScriptActivity.from_function(
    inputs={
        "content": extract_entities["content"],
    },
    depends_on=["extract_entities"],
)
async def detect_toxicity(content):
    # Simplified toxicity detection using keyword matching
    # In production, use a dedicated toxicity detection model

    toxic_keywords = ["idiot", "stupid", "garbage", "spam", "click here"]

    for item in content:
        text_lower = item["text"].lower()

        # Check for toxic keywords
        matches = [kw for kw in toxic_keywords if kw in text_lower]
        item["toxic_keywords_found"] = matches
        item["toxicity_score"] = len(matches) / 5.0  # Normalize to 0-1
        item["is_toxic"] = len(matches) > 0

    return {
        "content": content,
        "toxic_count": sum(1 for item in content if item["is_toxic"]),
    }


# ============================================================================
# PHASE 3: ML PREDICTIONS
# ============================================================================


# Step 9: Engineer features for ML models
@ScriptActivity.from_function(
    inputs={
        "content": detect_toxicity["content"],
    },
    depends_on=["detect_toxicity"],
)
async def engineer_features(content):
    for item in content:
        # Text-based features
        item["avg_word_length"] = sum(len(word) for word in item["text"].split()) / max(
            item["word_count"], 1
        )

        # Engagement signals (derived from sentiment and entities)
        item["sentiment_polarity"] = (
            1.0 if item["sentiment_label"] == "POSITIVE" else -1.0
        ) * item["sentiment_score"]

        # Complexity score
        item["complexity_score"] = (
            item["word_count"] * 0.3
            + item["entity_count"] * 0.5
            + item["avg_word_length"] * 0.2
        )

    return {
        "content": content,
    }


# Step 10: Predict engagement (likes, shares, comments)
@ScriptActivity.from_function(
    inputs={
        "content": engineer_features["content"],
    },
    depends_on=["engineer_features"],
)
async def predict_engagement(content):
    # Simplified engagement prediction model (in production, use trained ML model)
    for item in content:
        # Engagement score based on multiple factors
        base_score = 10

        # Positive sentiment increases engagement
        if item["sentiment_label"] == "POSITIVE":
            base_score += item["sentiment_score"] * 20

        # Complexity increases engagement (up to a point)
        base_score += min(item["complexity_score"], 15)

        # Entities increase engagement
        base_score += item["entity_count"] * 3

        # Toxicity decreases engagement
        base_score -= item["toxicity_score"] * 30

        # Platform factor
        if item["platform"] == "review":
            base_score *= 1.2

        item["predicted_engagement"] = max(0, round(base_score, 1))

    return {
        "content": content,
    }


# Step 11: Calculate quality score
@ScriptActivity.from_function(
    inputs={
        "content": predict_engagement["content"],
    },
    depends_on=["predict_engagement"],
)
async def score_quality(content):
    for item in content:
        # Quality score (0-100)
        quality = 50  # Base score

        # Length factor (not too short, not too long)
        if 50 <= item["word_count"] <= 200:
            quality += 15
        elif 20 <= item["word_count"] < 50:
            quality += 10
        elif item["word_count"] < 20:
            quality -= 10

        # Sentiment contributes to quality
        if item["sentiment_label"] == "POSITIVE":
            quality += 10
        elif item["sentiment_label"] == "NEGATIVE":
            quality += 5  # Negative but detailed feedback has value

        # Entities indicate informativeness
        quality += min(item["entity_count"] * 5, 20)

        # Toxicity severely reduces quality
        quality -= item["toxicity_score"] * 40

        item["quality_score"] = max(0, min(100, round(quality, 1)))

    return {
        "content": content,
    }


# ============================================================================
# PHASE 4: DATA ANALYTICS
# ============================================================================


# Step 12: Aggregate metrics with DuckDB
@ScriptActivity.from_function(
    inputs={
        "content": score_quality["content"],
    },
    depends_on=["score_quality"],
)
async def aggregate_metrics(content):
    import duckdb
    import pandas as pd

    df = pd.DataFrame(content)  # noqa: F841 - Used by duckdb.sql() queries below

    # Aggregate by platform
    platform_stats = duckdb.sql("""
        SELECT
            platform,
            COUNT(*) as content_count,
            AVG(quality_score) as avg_quality,
            AVG(predicted_engagement) as avg_engagement,
            SUM(CASE WHEN is_toxic THEN 1 ELSE 0 END) as toxic_count,
            AVG(sentiment_score) as avg_sentiment_score
        FROM df
        GROUP BY platform
        ORDER BY content_count DESC
    """).df()

    # Aggregate by sentiment
    sentiment_stats = duckdb.sql("""
        SELECT
            sentiment_label,
            COUNT(*) as count,
            AVG(quality_score) as avg_quality,
            AVG(toxicity_score) as avg_toxicity
        FROM df
        GROUP BY sentiment_label
    """).df()

    return {
        "platform_stats": platform_stats.to_dict(orient="records"),
        "sentiment_stats": sentiment_stats.to_dict(orient="records"),
        "content": content,
    }


# Step 13: Analyze trends
@ScriptActivity.from_function(
    inputs={
        "content": aggregate_metrics["content"],
    },
    depends_on=["aggregate_metrics"],
)
async def analyze_trends(content):
    import pandas as pd

    df = pd.DataFrame(content)

    # Quality distribution
    quality_bins = [0, 40, 70, 100]
    quality_labels = ["low", "medium", "high"]
    df["quality_tier"] = pd.cut(
        df["quality_score"], bins=quality_bins, labels=quality_labels
    )

    quality_distribution = df["quality_tier"].value_counts().to_dict()

    # Engagement distribution
    engagement_bins = [0, 10, 25, 100]
    engagement_labels = ["low", "medium", "high"]
    df["engagement_tier"] = pd.cut(
        df["predicted_engagement"], bins=engagement_bins, labels=engagement_labels
    )

    engagement_distribution = df["engagement_tier"].value_counts().to_dict()

    return {
        "quality_distribution": quality_distribution,
        "engagement_distribution": engagement_distribution,
        "avg_quality": round(df["quality_score"].mean(), 2),
        "avg_engagement": round(df["predicted_engagement"].mean(), 2),
    }


# Step 14: Export to data warehouse format (Parquet)
@ScriptActivity.from_function(
    inputs={
        "content": aggregate_metrics["content"],
        "platform_stats": aggregate_metrics["platform_stats"],
    },
    depends_on=["analyze_trends"],
)
async def export_warehouse(content, platform_stats):
    import io

    import pandas as pd
    import pyarrow as pa
    import pyarrow.parquet as pq

    # Prepare content data (remove embeddings for warehouse)
    content_clean = []
    for item in content:
        clean_item = {k: v for k, v in item.items() if k != "embedding"}
        content_clean.append(clean_item)

    content_df = pd.DataFrame(content_clean)
    platform_df = pd.DataFrame(platform_stats)

    # Convert to Parquet (in-memory)
    content_table = pa.Table.from_pandas(content_df)
    content_buffer = io.BytesIO()
    pq.write_table(content_table, content_buffer)

    platform_table = pa.Table.from_pandas(platform_df)
    platform_buffer = io.BytesIO()
    pq.write_table(platform_table, platform_buffer)

    return {
        "export_status": "success",
        "content_rows": len(content_df),
        "content_size_kb": round(len(content_buffer.getvalue()) / 1024, 2),
        "platform_rows": len(platform_df),
        "platform_size_kb": round(len(platform_buffer.getvalue()) / 1024, 2),
    }


# ============================================================================
# PHASE 5: MODERATION DECISION
# ============================================================================


# Step 15: Apply moderation rules
@ScriptActivity.from_function(
    inputs={
        "content": aggregate_metrics["content"],
    },
    depends_on=["aggregate_metrics"],
)
async def apply_moderation_rules(content):
    from pydantic import BaseModel

    class ModerationDecision(BaseModel):
        content_id: str
        decision: str  # approved, review, rejected
        reason: str
        confidence: float

    decisions = []

    for item in content:
        # Moderation logic
        if item["is_toxic"] or item["toxicity_score"] > 0.5:
            decision = "rejected"
            reason = "Toxic content detected"
            confidence = item["toxicity_score"]

        elif item["quality_score"] < 30:
            decision = "rejected"
            reason = "Low quality score"
            confidence = (100 - item["quality_score"]) / 100

        elif item["quality_score"] < 50 or item["sentiment_label"] == "NEGATIVE":
            decision = "review"
            reason = "Requires manual review"
            confidence = 0.6

        else:
            decision = "approved"
            reason = "Meets quality standards"
            confidence = item["quality_score"] / 100

        decisions.append(
            ModerationDecision(
                content_id=item["content_id"],
                decision=decision,
                reason=reason,
                confidence=round(confidence, 3),
            ).dict()
        )

    return {
        "decisions": decisions,
    }


# Step 16: Route content based on decisions
@ScriptActivity.from_function(
    inputs={
        "content": aggregate_metrics["content"],
        "decisions": apply_moderation_rules["decisions"],
    },
    depends_on=["apply_moderation_rules"],
)
async def route_content(content, decisions):
    # Create decision lookup
    decision_map = {d["content_id"]: d for d in decisions}

    # Route content
    approved = []
    review_queue = []
    rejected = []

    for item in content:
        decision = decision_map.get(item["content_id"])
        if not decision:
            continue

        content_summary = {
            "content_id": item["content_id"],
            "user_id": item["user_id"],
            "text": item["text"][:100] + "..."
            if len(item["text"]) > 100
            else item["text"],
            "platform": item["platform"],
            "quality_score": item["quality_score"],
            "predicted_engagement": item["predicted_engagement"],
            "decision": decision["decision"],
            "reason": decision["reason"],
        }

        if decision["decision"] == "approved":
            approved.append(content_summary)
        elif decision["decision"] == "review":
            review_queue.append(content_summary)
        else:
            rejected.append(content_summary)

    return {
        "approved": approved,
        "review_queue": review_queue,
        "rejected": rejected,
        "summary": {
            "approved_count": len(approved),
            "review_count": len(review_queue),
            "rejected_count": len(rejected),
            "total_processed": len(content),
        },
    }


# Build the comprehensive workflow
content_moderation_workflow = Workflow(
    name="content_moderation_system",
    activities=[
        # Phase 1: Ingestion
        fetch_content,
        validate_schema,
        enrich_metadata,
        # Phase 2: NLP Analysis
        count_tokens,
        generate_embeddings,
        analyze_sentiment,
        extract_entities,
        detect_toxicity,
        # Phase 3: ML Predictions
        engineer_features,
        predict_engagement,
        score_quality,
        # Phase 4: Analytics
        aggregate_metrics,
        analyze_trends,
        export_warehouse,
        # Phase 5: Moderation
        apply_moderation_rules,
        route_content,
    ],
)

if __name__ == "__main__":
    # Print the compiled YAML to verify
    print(content_moderation_workflow)
