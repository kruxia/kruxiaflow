"""Pydantic schemas for error responses."""

from typing import Any

from pydantic import BaseModel, Field


class ErrorDetail(BaseModel):
    """Detailed error information."""

    type: str = Field(..., description="Error type (e.g., ValidationError, NotFoundError)")
    message: str = Field(..., description="Human-readable error message")
    field: str | None = Field(None, description="Field that caused the error (if applicable)")
    code: str | None = Field(None, description="Error code for programmatic handling")

    class Config:
        extra = "allow"


class ErrorResponse(BaseModel):
    """Standard error response format."""

    error: str = Field(..., description="Error message")
    errors: list[ErrorDetail] | None = Field(
        None,
        description="Detailed error information",
    )
    status_code: int | None = Field(None, description="HTTP status code")
    request_id: str | None = Field(None, description="Request identifier for debugging")

    class Config:
        extra = "allow"


class ValidationError(BaseModel):
    """Validation error details."""

    field: str = Field(..., description="Field that failed validation")
    message: str = Field(..., description="Validation error message")
    value: Any | None = Field(None, description="Invalid value that was provided")
    constraint: str | None = Field(None, description="Constraint that was violated")


class ValidationErrorResponse(ErrorResponse):
    """Error response for validation failures."""

    validation_errors: list[ValidationError] = Field(
        default_factory=list,
        description="List of validation errors",
    )
