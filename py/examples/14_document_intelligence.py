"""Document Intelligence Pipeline - py-std worker example.

This example demonstrates py-std worker NLP capabilities:
- Text embeddings with sentence-transformers
- Sentiment analysis with transformers
- Named Entity Recognition with spacy
- Token counting with tiktoken
- Document clustering and semantic similarity

The workflow processes research paper abstracts, generates embeddings,
analyzes sentiment, extracts entities, and clusters similar documents.
"""

from kruxiaflow import ScriptActivity, Workflow


# Step 1: Preprocess and load documents
@ScriptActivity.from_function(
    inputs={
        # Sample research paper abstracts
        "documents": [
            {
                "id": "paper_1",
                "title": "Deep Learning for Natural Language Processing",
                "abstract": "This paper presents a novel approach to NLP using transformer architectures. We demonstrate significant improvements in sentiment analysis and named entity recognition tasks.",
            },
            {
                "id": "paper_2",
                "title": "Machine Learning in Healthcare: A Review",
                "abstract": "We review recent advances in applying machine learning to healthcare problems. Our analysis shows promising results in disease prediction and patient outcome modeling.",
            },
            {
                "id": "paper_3",
                "title": "Efficient Training of Large Language Models",
                "abstract": "This work introduces techniques for efficient training of billion-parameter language models. We achieve 40% reduction in training time while maintaining model quality.",
            },
            {
                "id": "paper_4",
                "title": "Computer Vision for Medical Imaging",
                "abstract": "We apply deep convolutional neural networks to medical image analysis. Our model achieves state-of-the-art performance in tumor detection and classification.",
            },
            {
                "id": "paper_5",
                "title": "Sentiment Analysis on Social Media Data",
                "abstract": "This study analyzes sentiment patterns in social media posts using transformer-based models. We identify key factors influencing public opinion on various topics.",
            },
        ],
    },
)
async def preprocess_documents(documents):
    import re

    # Clean and normalize text
    def clean_text(text):
        # Remove extra whitespace
        text = re.sub(r"\s+", " ", text)
        # Normalize quotes
        text = text.replace('"', '"').replace('"', '"')
        # Normalize dashes
        text = text.replace("\u2013", "-").replace("\u2014", "-")  # EN DASH and EM DASH
        return text.strip()

    for doc in documents:
        doc["abstract_clean"] = clean_text(doc["abstract"])
        doc["title_clean"] = clean_text(doc["title"])
        # Combine title and abstract for full context
        doc["full_text"] = f"{doc['title_clean']}. {doc['abstract_clean']}"

    return {
        "documents": documents,
        "document_count": len(documents),
    }


# Step 2: Count tokens for LLM API cost estimation
@ScriptActivity.from_function(
    inputs={
        "documents": preprocess_documents["documents"],
    },
    depends_on=["preprocess_documents"],
)
async def tokenize_count(documents):
    import tiktoken

    # Use cl100k_base encoding (GPT-4, GPT-3.5-turbo)
    encoding = tiktoken.get_encoding("cl100k_base")

    total_tokens = 0
    for doc in documents:
        tokens = encoding.encode(doc["full_text"])
        doc["token_count"] = len(tokens)
        total_tokens += len(tokens)

    # Estimate API costs (example pricing)
    cost_per_1k_tokens = 0.03  # Example: $0.03 per 1K tokens
    estimated_cost = (total_tokens / 1000) * cost_per_1k_tokens

    return {
        "documents": documents,
        "total_tokens": total_tokens,
        "avg_tokens_per_doc": round(total_tokens / len(documents), 1),
        "estimated_api_cost_usd": round(estimated_cost, 4),
    }


# Step 3: Generate text embeddings for semantic similarity
@ScriptActivity.from_function(
    inputs={
        "documents": tokenize_count["documents"],
    },
    depends_on=["tokenize_count"],
)
async def generate_embeddings(documents):
    from sentence_transformers import SentenceTransformer

    # Load pre-trained sentence transformer model
    # Model will be downloaded on first use (cached for subsequent runs)
    model = SentenceTransformer("all-MiniLM-L6-v2")

    # Generate embeddings for all documents (batch processing)
    texts = [doc["full_text"] for doc in documents]
    embeddings = model.encode(texts, show_progress_bar=False)

    # Add embeddings to documents
    for i, doc in enumerate(documents):
        doc["embedding"] = embeddings[i].tolist()

    return {
        "documents": documents,
        "embedding_dimension": embeddings.shape[1],
        "model_name": "all-MiniLM-L6-v2",
    }


# Step 4: Perform sentiment analysis on abstracts
@ScriptActivity.from_function(
    inputs={
        "documents": generate_embeddings["documents"],
    },
    depends_on=["generate_embeddings"],
)
async def analyze_sentiment(documents):
    from transformers import pipeline

    # Load sentiment analysis pipeline
    # Using distilbert-base-uncased-finetuned-sst-2-english
    sentiment_analyzer = pipeline(
        "sentiment-analysis",
        model="distilbert-base-uncased-finetuned-sst-2-english",
    )

    # Analyze sentiment for each abstract
    for doc in documents:
        result = sentiment_analyzer(doc["abstract_clean"])[0]
        doc["sentiment"] = result["label"]
        doc["sentiment_score"] = round(result["score"], 4)

    # Calculate overall statistics
    positive_count = sum(1 for d in documents if d["sentiment"] == "POSITIVE")
    negative_count = sum(1 for d in documents if d["sentiment"] == "NEGATIVE")

    return {
        "documents": documents,
        "sentiment_distribution": {
            "positive": positive_count,
            "negative": negative_count,
        },
        "avg_confidence": round(
            sum(d["sentiment_score"] for d in documents) / len(documents), 4
        ),
    }


# Step 5: Extract named entities using spacy
@ScriptActivity.from_function(
    inputs={
        "documents": analyze_sentiment["documents"],
    },
    depends_on=["analyze_sentiment"],
)
async def extract_entities(documents):
    import spacy

    # Load spacy model (pre-installed in py-nlp worker)
    nlp = spacy.load("en_core_web_sm")

    # Extract entities from each document
    all_entities = []
    for doc in documents:
        # Process text with spacy
        spacy_doc = nlp(doc["full_text"])

        # Extract entities
        entities = []
        for ent in spacy_doc.ents:
            entities.append(
                {
                    "text": ent.text,
                    "label": ent.label_,
                    "start": ent.start_char,
                    "end": ent.end_char,
                }
            )

        doc["entities"] = entities
        all_entities.extend(entities)

    # Count entity types
    entity_type_counts = {}
    for ent in all_entities:
        label = ent["label"]
        entity_type_counts[label] = entity_type_counts.get(label, 0) + 1

    # Extract unique entity texts by type
    entity_texts_by_type = {}
    for ent in all_entities:
        label = ent["label"]
        if label not in entity_texts_by_type:
            entity_texts_by_type[label] = set()
        entity_texts_by_type[label].add(ent["text"])

    # Convert sets to sorted lists for output
    entity_texts_by_type = {k: sorted(v) for k, v in entity_texts_by_type.items()}

    return {
        "documents": documents,
        "total_entities": len(all_entities),
        "entity_type_counts": entity_type_counts,
        "unique_entities_by_type": entity_texts_by_type,
    }


# Step 6: Calculate document similarity and clustering
@ScriptActivity.from_function(
    inputs={
        "documents": extract_entities["documents"],
    },
    depends_on=["extract_entities"],
)
async def cluster_documents(documents):
    import numpy as np
    from sklearn.cluster import KMeans
    from sklearn.metrics.pairwise import cosine_similarity

    # Extract embeddings
    embeddings = np.array([doc["embedding"] for doc in documents])

    # Calculate pairwise cosine similarity
    similarity_matrix = cosine_similarity(embeddings)

    # Find most similar document pairs
    similarities = []
    for i in range(len(documents)):
        for j in range(i + 1, len(documents)):
            similarities.append(
                {
                    "doc1_id": documents[i]["id"],
                    "doc2_id": documents[j]["id"],
                    "similarity": round(float(similarity_matrix[i, j]), 4),
                }
            )

    # Sort by similarity
    similarities.sort(key=lambda x: x["similarity"], reverse=True)
    top_similar_pairs = similarities[:3]

    # Perform K-means clustering (k=2 for this small example)
    n_clusters = min(2, len(documents))
    kmeans = KMeans(n_clusters=n_clusters, random_state=42)
    cluster_labels = kmeans.fit_predict(embeddings)

    # Add cluster labels to documents
    for i, doc in enumerate(documents):
        doc["cluster"] = int(cluster_labels[i])

    # Group documents by cluster
    clusters = {}
    for doc in documents:
        cluster_id = doc["cluster"]
        if cluster_id not in clusters:
            clusters[cluster_id] = []
        clusters[cluster_id].append(
            {
                "id": doc["id"],
                "title": doc["title"],
            }
        )

    return {
        "documents": documents,
        "top_similar_pairs": top_similar_pairs,
        "clusters": clusters,
        "num_clusters": n_clusters,
    }


# Step 7: Generate document summaries and insights
@ScriptActivity.from_function(
    inputs={
        "documents": cluster_documents["documents"],
        "entity_counts": extract_entities["entity_type_counts"],
        "top_similar_pairs": cluster_documents["top_similar_pairs"],
        "clusters": cluster_documents["clusters"],
    },
    depends_on=["cluster_documents"],
)
async def generate_insights(documents, entity_counts, top_similar_pairs, clusters):
    import json

    # Create summary for each document
    for doc in documents:
        # Count key topics based on entities
        entity_count = len(doc["entities"])
        topics = list({ent["label"] for ent in doc["entities"]})

        doc["summary"] = {
            "token_count": doc["token_count"],
            "sentiment": doc["sentiment"],
            "entity_count": entity_count,
            "topics": topics,
            "cluster": doc["cluster"],
        }

    # Overall collection insights
    insights = {
        "total_documents": len(documents),
        "total_tokens": sum(d["token_count"] for d in documents),
        "entity_distribution": entity_counts,
        "sentiment_summary": {
            "positive": sum(1 for d in documents if d["sentiment"] == "POSITIVE"),
            "negative": sum(1 for d in documents if d["sentiment"] == "NEGATIVE"),
        },
        "most_similar_documents": top_similar_pairs,
        "cluster_distribution": {str(k): len(v) for k, v in clusters.items()},
    }

    # Identify key themes across clusters
    cluster_themes = {}
    for cluster_id, docs_in_cluster in clusters.items():
        doc_ids = [d["id"] for d in docs_in_cluster]
        cluster_docs = [d for d in documents if d["id"] in doc_ids]

        # Extract common topics
        all_entities = []
        for doc in cluster_docs:
            all_entities.extend([e["label"] for e in doc["entities"]])

        # Count entity types in this cluster
        entity_counts_cluster = {}
        for entity_type in all_entities:
            entity_counts_cluster[entity_type] = (
                entity_counts_cluster.get(entity_type, 0) + 1
            )

        # Get top 3 topics
        top_topics = sorted(
            entity_counts_cluster.items(), key=lambda x: x[1], reverse=True
        )[:3]

        cluster_themes[f"cluster_{cluster_id}"] = {
            "document_count": len(cluster_docs),
            "top_topics": [t[0] for t in top_topics],
            "documents": [d["title"] for d in docs_in_cluster],
        }

    insights["cluster_themes"] = cluster_themes

    return {
        "document_summaries": [
            {
                "id": d["id"],
                "title": d["title"],
                "summary": d["summary"],
            }
            for d in documents
        ],
        "collection_insights": insights,
        "insights_json": json.dumps(insights, indent=2),
    }


# Build the workflow
document_intelligence_workflow = Workflow(
    name="document_intelligence",
    activities=[
        preprocess_documents,
        tokenize_count,
        generate_embeddings,
        analyze_sentiment,
        extract_entities,
        cluster_documents,
        generate_insights,
    ],
)

if __name__ == "__main__":
    # Print the compiled YAML to verify
    print(document_intelligence_workflow)
