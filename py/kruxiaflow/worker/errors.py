"""Worker SDK error types."""


class WorkerError(Exception):
    """Base class for worker errors."""


class ConfigError(WorkerError):
    """Configuration error."""


class AuthenticationError(WorkerError):
    """Authentication failed."""


class ActivityNotFoundError(WorkerError):
    """Activity implementation not found in registry."""


class ActivityTimeoutError(WorkerError):
    """Activity execution timed out."""


class ActivityExecutionError(WorkerError):
    """Activity execution failed."""


class FileOperationError(WorkerError):
    """File upload/download operation failed."""
