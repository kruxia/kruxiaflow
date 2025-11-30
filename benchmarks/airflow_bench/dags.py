"""Airflow 3 DAG definitions for benchmarking"""

import pendulum
from airflow import DAG
from airflow.operators.python import PythonOperator


def echo_task(**context):
    """Simple echo task that returns its input"""
    return context.get("params", {})


default_args = {
    "owner": "benchmark",
    "depends_on_past": False,
    "email_on_failure": False,
    "email_on_retry": False,
    "retries": 0,
}


# Sequential benchmark with 5 activities
with DAG(
    dag_id="sequential_bench_5",
    default_args=default_args,
    start_date=pendulum.datetime(2024, 1, 1, tz="UTC"),
    schedule=None,
    catchup=False,
    max_active_runs=1000,  # Allow many concurrent runs for benchmarking
) as sequential_bench_5:
    tasks = []
    for i in range(5):
        task = PythonOperator(
            task_id=f"echo_{i}",
            python_callable=echo_task,
            params={"activity": i},
        )
        if tasks:
            tasks[-1] >> task
        tasks.append(task)


# Sequential benchmark with 3 activities
with DAG(
    dag_id="sequential_bench_3",
    default_args=default_args,
    start_date=pendulum.datetime(2024, 1, 1, tz="UTC"),
    schedule=None,
    catchup=False,
    max_active_runs=1000,  # Allow many concurrent runs for benchmarking
) as sequential_bench_3:
    tasks = []
    for i in range(3):
        task = PythonOperator(
            task_id=f"echo_{i}",
            python_callable=echo_task,
            params={"activity": i},
        )
        if tasks:
            tasks[-1] >> task
        tasks.append(task)


# Parallel benchmark with 10 activities
with DAG(
    dag_id="parallel_bench_10",
    default_args=default_args,
    start_date=pendulum.datetime(2024, 1, 1, tz="UTC"),
    schedule=None,
    catchup=False,
    max_active_runs=1000,  # Allow many concurrent runs for benchmarking
) as parallel_bench_10:
    start = PythonOperator(
        task_id="start",
        python_callable=echo_task,
        params={"stage": "start"},
    )

    parallel_tasks = []
    for i in range(10):
        task = PythonOperator(
            task_id=f"parallel_{i}",
            python_callable=echo_task,
            params={"activity": i},
        )
        start >> task
        parallel_tasks.append(task)

    end = PythonOperator(
        task_id="end",
        python_callable=echo_task,
        params={"stage": "end"},
    )

    for task in parallel_tasks:
        task >> end
