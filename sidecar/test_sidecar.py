"""
Tests for the Mem0 sidecar FastAPI app.

Run: cd sidecar && source .venv/bin/activate && python -m pytest test_sidecar.py -v

Requires a running sidecar (or starts one in-process via TestClient).
"""

import os
import pytest
from fastapi.testclient import TestClient

os.environ.setdefault("AWS_PROFILE", "bedrock-beta")
os.environ.setdefault("AWS_REGION", "us-east-1")

from mem0_sidecar import app


@pytest.fixture(scope="module")
def client():
    app.state.data_dir = "/tmp/panes_sidecar_test"
    app.state.llm_provider = "aws_bedrock"
    app.state.llm_model = "us.anthropic.claude-sonnet-4-6"
    with TestClient(app) as c:
        yield c


def test_health(client):
    resp = client.get("/health")
    assert resp.status_code == 200
    data = resp.json()
    assert data["status"] == "ok"
    assert data["mem0_ready"] is True


def test_add_memories(client):
    resp = client.post(
        "/v1/memories/",
        json={
            "transcript": (
                "User: Always use pnpm not npm.\n"
                "Assistant: Got it, pnpm for everything."
            ),
            "user_id": "ws:pytest-test",
            "thread_id": "pytest-t1",
        },
    )
    assert resp.status_code == 200
    data = resp.json()
    results = data.get("results", [])
    assert len(results) >= 1, f"Expected extracted memories, got: {data}"
    assert any("pnpm" in r.get("memory", "").lower() for r in results)


def test_search(client):
    resp = client.post(
        "/v1/memories/search/",
        json={
            "query": "package manager",
            "user_id": "ws:pytest-test",
            "limit": 5,
        },
    )
    assert resp.status_code == 200
    data = resp.json()
    results = data.get("results", [])
    assert len(results) >= 1, f"Search should find pnpm memory, got: {data}"


def test_get_all(client):
    resp = client.get("/v1/memories/", params={"user_id": "ws:pytest-test"})
    assert resp.status_code == 200
    data = resp.json()
    results = data.get("results", [])
    assert len(results) >= 1


def test_delete(client):
    # Get a memory to delete
    resp = client.get("/v1/memories/", params={"user_id": "ws:pytest-test"})
    results = resp.json().get("results", [])
    if not results:
        pytest.skip("No memories to delete")

    memory_id = results[0]["id"]
    del_resp = client.delete(f"/v1/memories/{memory_id}/")
    assert del_resp.status_code == 200


def test_dedup(client):
    """Adding the same info twice should not create duplicates."""
    # Add first transcript
    client.post(
        "/v1/memories/",
        json={
            "transcript": "User: Use Tailwind CSS.\nAssistant: Tailwind it is.",
            "user_id": "ws:pytest-dedup",
            "thread_id": "dedup-t1",
        },
    )
    count1 = len(
        client.get("/v1/memories/", params={"user_id": "ws:pytest-dedup"})
        .json()
        .get("results", [])
    )

    # Add overlapping transcript
    client.post(
        "/v1/memories/",
        json={
            "transcript": "User: Remember, Tailwind CSS.\nAssistant: Yes, Tailwind.",
            "user_id": "ws:pytest-dedup",
            "thread_id": "dedup-t2",
        },
    )
    count2 = len(
        client.get("/v1/memories/", params={"user_id": "ws:pytest-dedup"})
        .json()
        .get("results", [])
    )

    assert count2 <= count1 + 1, f"Dedup should prevent growth: {count1} -> {count2}"

    # Cleanup
    for m in (
        client.get("/v1/memories/", params={"user_id": "ws:pytest-dedup"})
        .json()
        .get("results", [])
    ):
        client.delete(f"/v1/memories/{m['id']}/")
