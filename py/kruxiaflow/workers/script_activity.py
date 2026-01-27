"""Script activity for executing arbitrary Python code.

This activity is shared by all standard Python workers (std, data, ml, nlp).
The only difference between workers is which packages are pre-installed.
"""

import traceback
from typing import Any

from kruxiaflow.worker import ActivityContext, ActivityResult, activity


@activity(name="script")
async def script_activity(
    params: dict[str, Any], ctx: ActivityContext
) -> ActivityResult:
    """
    Execute arbitrary Python script.

    Parameters:
        script: str - Python code to execute
        inputs: dict - Input data available as INPUT variable

    The script has access to:
        INPUT: dict
            Input data from workflow parameters.
        OUTPUT: dict
            Set this to return output data. Initialize as empty dict.
        ctx: ActivityContext
            Context for heartbeat, file operations, logging.
        logger: logging.Logger
            Logger for this activity (shortcut for ctx.logger).
        workflow_id: str
            UUID of the current workflow.
        activity_key: str
            Key of this activity in the workflow.

    Example script:
        ```python
        import pandas as pd

        df = pd.DataFrame(INPUT["records"])
        df_clean = df.dropna()

        OUTPUT = {"row_count": len(df_clean)}
        ```
    """
    script_code = params.get("script", "")
    inputs = params.get("inputs", {})

    if not script_code:
        return ActivityResult.error(
            message="No script provided",
            code="MISSING_SCRIPT",
            retryable=False,
        )

    # Build execution namespace
    namespace: dict[str, Any] = {
        # Input/output
        "INPUT": inputs,
        "OUTPUT": {},
        # Context
        "ctx": ctx,
        "logger": ctx.logger,
        "workflow_id": str(ctx.workflow_id),
        "activity_key": ctx.activity_key,
        # Allow all builtins (imports, etc.)
        "__builtins__": __builtins__,
    }

    try:
        # Compile and execute
        code = compile(script_code, "<script>", "exec")
        exec(code, namespace)

        output = namespace.get("OUTPUT", {})
        return ActivityResult.value("result", output)

    except SyntaxError as e:
        # Syntax errors get special formatting
        ctx.logger.error(f"Script syntax error: {e}")
        return ActivityResult.error(
            message=f"SyntaxError at line {e.lineno}: {e.msg}",
            code="SYNTAX_ERROR",
            retryable=False,
        )

    except Exception as e:
        # Include traceback for debugging
        tb = traceback.format_exc()
        ctx.logger.error(f"Script execution failed:\n{tb}")

        return ActivityResult.error(
            message=f"{type(e).__name__}: {e}",
            code="SCRIPT_ERROR",
            retryable=False,
        )
