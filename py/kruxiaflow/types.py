"""Common annotated types for field validation.

These types provide consistent validation patterns across the SDK.
"""

from typing import Annotated

from pydantic import Field

# Pattern for valid identifiers (activity keys, activity names)
# Alphanumeric, underscore, hyphen - must be non-empty
IDENTIFIER_PATTERN = r"^[a-zA-Z0-9_-]+$"

# Pattern for worker slugs (lowercase, starts with letter)
WORKER_SLUG_PATTERN = r"^[a-z][a-z0-9_-]*$"

# Pattern for filenames (allows subdirectories, must start with alphanumeric/underscore)
FILENAME_PATTERN = r"^[a-zA-Z0-9_][a-zA-Z0-9_./-]*$"


# Activity key - used for activity identifiers within workflows
ActivityKey = Annotated[str, Field(min_length=1, pattern=IDENTIFIER_PATTERN)]

# Activity name - the name of the activity type to execute (required)
ActivityName = Annotated[str, Field(min_length=1, pattern=IDENTIFIER_PATTERN)]

# Activity name that allows empty for fluent API (validated at serialization)
ActivityNameOptional = Annotated[str, Field(default="", pattern=r"^[a-zA-Z0-9_-]*$")]

# Worker name - simple non-empty string for worker field in activities (with default)
WorkerName = Annotated[str, Field(default="std", min_length=1)]

# Worker name - required (no default), for use in API responses
WorkerNameRequired = Annotated[str, Field(min_length=1)]

# Worker slug - lowercase identifier for worker config (starts with letter)
WorkerSlug = Annotated[str, Field(min_length=1, pattern=WORKER_SLUG_PATTERN)]

# Filename - allows subdirectories, prevents path traversal
Filename = Annotated[str, Field(min_length=1, pattern=FILENAME_PATTERN)]
