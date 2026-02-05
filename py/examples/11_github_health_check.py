"""GitHub Repository Health Check - py-std worker example.

This example demonstrates py-std worker capabilities:
- HTTP API calls with httpx
- JSON schema validation with pydantic
- Date parsing and manipulation with python-dateutil
- Fast JSON processing with orjson
- YAML configuration parsing with pyyaml

The workflow fetches GitHub repository data, validates the structure,
analyzes activity metrics, and generates a health score report.
"""

from kruxiaflow import ScriptActivity, Workflow


# Step 1: Fetch repository metadata from GitHub API
# Uses httpx for HTTP client functionality
@ScriptActivity.from_function(
    inputs={
        "owner": "anthropics",
        "repo": "anthropic-sdk-python",
    },
)
async def fetch_repo(owner, repo):
    from datetime import datetime

    import httpx

    # Fetch repository data from GitHub API
    repo_url = f"https://api.github.com/repos/{owner}/{repo}"
    headers = {"Accept": "application/vnd.github.v3+json"}

    async with httpx.AsyncClient() as client:
        response = await client.get(repo_url, headers=headers)
        response.raise_for_status()
        repo_data = response.json()

    return {
        "repo_data": repo_data,
        "fetched_at": datetime.utcnow().isoformat(),
    }


# Step 2: Validate repository data structure using Pydantic
# Ensures the API response has expected fields and types
@ScriptActivity.from_function(
    inputs={
        "repo_data": fetch_repo["repo_data"],
    },
    depends_on=["fetch_repo"],
)
async def validate_structure(repo_data):
    from pydantic import BaseModel, ValidationError

    # Define expected schema for GitHub repository data
    class GitHubRepoSchema(BaseModel):
        id: int
        name: str
        full_name: str
        description: str | None
        stargazers_count: int
        forks_count: int
        open_issues_count: int
        created_at: str
        updated_at: str
        pushed_at: str
        size: int
        language: str | None

        class Config:
            extra = "allow"  # Allow additional fields from GitHub API

    try:
        # Validate the repository data
        validated = GitHubRepoSchema(**repo_data)

        return {
            "valid": True,
            "repo_name": validated.full_name,
            "stars": validated.stargazers_count,
            "forks": validated.forks_count,
            "open_issues": validated.open_issues_count,
            "created_at": validated.created_at,
            "updated_at": validated.updated_at,
            "pushed_at": validated.pushed_at,
            "size_kb": validated.size,
            "language": validated.language,
        }
    except ValidationError as e:
        return {
            "valid": False,
            "errors": e.errors(),
        }


# Step 3: Parse and analyze dates to calculate repository age and activity
# Uses python-dateutil for robust date parsing
@ScriptActivity.from_function(
    inputs={
        "created_at": validate_structure["created_at"],
        "updated_at": validate_structure["updated_at"],
        "pushed_at": validate_structure["pushed_at"],
    },
    depends_on=["validate_structure"],
)
async def parse_dates(created_at, updated_at, pushed_at):
    from datetime import datetime, timezone

    from dateutil import parser

    # Parse ISO 8601 dates from GitHub API
    created = parser.isoparse(created_at)
    updated = parser.isoparse(updated_at)
    pushed = parser.isoparse(pushed_at)
    now = datetime.now(timezone.utc)

    # Calculate time deltas
    age_days = (now - created).days
    days_since_update = (now - updated).days
    days_since_push = (now - pushed).days

    return {
        "age_days": age_days,
        "age_years": round(age_days / 365.25, 1),
        "days_since_update": days_since_update,
        "days_since_push": days_since_push,
        "created_date": created.strftime("%Y-%m-%d"),
        "last_push_date": pushed.strftime("%Y-%m-%d"),
    }


# Step 4: Fetch recent commit activity
@ScriptActivity.from_function(
    inputs={
        "owner": "anthropics",
        "repo": "anthropic-sdk-python",
    },
    depends_on=["validate_structure"],
)
async def fetch_commits(owner, repo):
    from datetime import datetime, timedelta, timezone

    import httpx

    # Fetch commits from the last 30 days
    since_date = (datetime.now(timezone.utc) - timedelta(days=30)).isoformat()
    commits_url = f"https://api.github.com/repos/{owner}/{repo}/commits"
    headers = {"Accept": "application/vnd.github.v3+json"}
    params = {"since": since_date, "per_page": 100}

    async with httpx.AsyncClient() as client:
        response = await client.get(commits_url, headers=headers, params=params)
        response.raise_for_status()
        commits = response.json()

    # Calculate commit statistics
    commit_count = len(commits)
    unique_authors = len(
        {c["commit"]["author"]["name"] for c in commits if "commit" in c}
    )

    return {
        "commit_count_30d": commit_count,
        "unique_authors_30d": unique_authors,
        "commits_per_day": round(commit_count / 30, 2),
    }


# Step 5: Calculate repository health score
# Combines multiple metrics into a single health score
@ScriptActivity.from_function(
    inputs={
        "stars": validate_structure["stars"],
        "forks": validate_structure["forks"],
        "open_issues": validate_structure["open_issues"],
        "days_since_push": parse_dates["days_since_push"],
        "commit_count_30d": fetch_commits["commit_count_30d"],
        "unique_authors_30d": fetch_commits["unique_authors_30d"],
    },
    depends_on=["parse_dates", "fetch_commits"],
)
async def calculate_health(
    stars, forks, open_issues, days_since_push, commit_count_30d, unique_authors_30d
):
    # Health score calculation based on multiple factors
    # Scale: 0-100, higher is better

    # Activity score (0-30 points): Recent commits indicate active development
    activity_score = 0
    if days_since_push <= 7:
        activity_score = 30
    elif days_since_push <= 30:
        activity_score = 20
    elif days_since_push <= 90:
        activity_score = 10

    # Engagement score (0-30 points): Stars and forks indicate community interest
    engagement_score = min(30, (stars // 100) + (forks // 20))

    # Maintenance score (0-20 points): Active commits and contributors
    maintenance_score = min(20, (commit_count_30d // 5) + (unique_authors_30d * 2))

    # Issue management score (0-20 points): Lower open issues is better
    issue_score = max(0, 20 - (open_issues // 10))

    # Calculate total health score
    total_score = activity_score + engagement_score + maintenance_score + issue_score

    # Determine health status
    if total_score >= 80:
        status = "excellent"
    elif total_score >= 60:
        status = "good"
    elif total_score >= 40:
        status = "fair"
    else:
        status = "needs_attention"

    return {
        "health_score": total_score,
        "status": status,
        "breakdown": {
            "activity": activity_score,
            "engagement": engagement_score,
            "maintenance": maintenance_score,
            "issue_management": issue_score,
        },
    }


# Step 6: Format final report using orjson for fast JSON serialization
@ScriptActivity.from_function(
    inputs={
        "repo_name": validate_structure["repo_name"],
        "language": validate_structure["language"],
        "stars": validate_structure["stars"],
        "forks": validate_structure["forks"],
        "age_years": parse_dates["age_years"],
        "days_since_push": parse_dates["days_since_push"],
        "commit_count_30d": fetch_commits["commit_count_30d"],
        "health_score": calculate_health["health_score"],
        "status": calculate_health["status"],
        "breakdown": calculate_health["breakdown"],
    },
    depends_on=["calculate_health"],
)
async def format_report(
    repo_name,
    language,
    stars,
    forks,
    age_years,
    days_since_push,
    commit_count_30d,
    health_score,
    status,
    breakdown,
):
    from datetime import datetime

    import orjson

    # Build comprehensive health report
    report = {
        "repository": repo_name,
        "language": language,
        "metrics": {
            "stars": stars,
            "forks": forks,
            "age_years": age_years,
            "days_since_last_push": days_since_push,
            "commits_last_30_days": commit_count_30d,
        },
        "health": {
            "score": health_score,
            "status": status,
            "breakdown": breakdown,
        },
        "generated_at": datetime.utcnow().isoformat(),
    }

    # Serialize using orjson (faster than standard json library)
    # orjson returns bytes, so decode to string for return
    report_json = orjson.dumps(report, option=orjson.OPT_INDENT_2).decode()

    return {
        "report_json": report_json,
        "report": report,
        "summary": f"{repo_name}: Health Score {health_score}/100 ({status})",
    }


# Build the workflow
github_health_workflow = Workflow(
    name="github_health_check",
    activities=[
        fetch_repo,
        validate_structure,
        parse_dates,
        fetch_commits,
        calculate_health,
        format_report,
    ],
)

if __name__ == "__main__":
    # Print the compiled YAML to verify
    print(github_health_workflow)
