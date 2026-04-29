"""
Panes Mem0 Sidecar — FastAPI wrapper around Mem0 for memory extraction and retrieval.

Runs as a local HTTP server. Panes spawns this as a child process and communicates via REST.
Uses huggingface embeddings (local, CPU-only) and Qdrant in embedded mode.
LLM calls route through the user's configured provider (Bedrock by default).
"""

import argparse
import logging
import os
import sys
from contextlib import asynccontextmanager
from typing import Optional

from fastapi import FastAPI, HTTPException
from pydantic import BaseModel

logger = logging.getLogger("mem0_sidecar")

mem0_instance = None


def build_config(data_dir: str, llm_provider: str, llm_model: str) -> dict:
    return {
        "vector_store": {
            "provider": "qdrant",
            "config": {
                "collection_name": "panes_memories",
                "path": os.path.join(data_dir, "qdrant"),
                "embedding_model_dims": 384,
            },
        },
        "embedder": {
            "provider": "huggingface",
            "config": {"model": "all-MiniLM-L6-v2"},
        },
        "llm": {
            "provider": llm_provider,
            "config": {
                "model": llm_model,
                "aws_region": os.environ.get("AWS_REGION", "us-east-1"),
            },
        },
        "history_db_path": os.path.join(data_dir, "history.db"),
        "version": "v1.1",
    }


@asynccontextmanager
async def lifespan(app: FastAPI):
    global mem0_instance
    from mem0 import Memory

    data_dir = app.state.data_dir
    llm_provider = app.state.llm_provider
    llm_model = app.state.llm_model

    logger.info("Initializing Mem0 (data_dir=%s, llm=%s/%s)", data_dir, llm_provider, llm_model)
    os.makedirs(data_dir, exist_ok=True)

    config = build_config(data_dir, llm_provider, llm_model)
    mem0_instance = Memory.from_config(config_dict=config)
    logger.info("Mem0 ready")
    yield
    mem0_instance = None


app = FastAPI(title="Panes Mem0 Sidecar", lifespan=lifespan)


class AddRequest(BaseModel):
    transcript: str
    user_id: str
    thread_id: str
    workspace_id: Optional[str] = None


class SearchRequest(BaseModel):
    query: str
    user_id: str
    limit: int = 10


class UpdateRequest(BaseModel):
    data: str


@app.get("/health")
def health():
    return {"status": "ok", "mem0_ready": mem0_instance is not None}


@app.post("/v1/memories/")
def add_memories(req: AddRequest):
    if not mem0_instance:
        raise HTTPException(503, "Mem0 not initialized")
    metadata = {"thread_id": req.thread_id}
    if req.workspace_id:
        metadata["workspace_id"] = req.workspace_id
    result = mem0_instance.add(
        req.transcript,
        user_id=req.user_id,
        metadata=metadata,
    )
    return result


@app.post("/v1/memories/search/")
def search_memories(req: SearchRequest):
    if not mem0_instance:
        raise HTTPException(503, "Mem0 not initialized")
    result = mem0_instance.search(
        req.query,
        filters={"user_id": req.user_id},
        limit=req.limit,
    )
    return result


@app.get("/v1/memories/")
def get_all_memories(user_id: str):
    if not mem0_instance:
        raise HTTPException(503, "Mem0 not initialized")
    result = mem0_instance.get_all(filters={"user_id": user_id})
    return result


@app.put("/v1/memories/{memory_id}/")
def update_memory(memory_id: str, req: UpdateRequest):
    if not mem0_instance:
        raise HTTPException(503, "Mem0 not initialized")
    result = mem0_instance.update(memory_id, req.data)
    return result


@app.delete("/v1/memories/{memory_id}/")
def delete_memory(memory_id: str):
    if not mem0_instance:
        raise HTTPException(503, "Mem0 not initialized")
    result = mem0_instance.delete(memory_id)
    return result


def main():
    parser = argparse.ArgumentParser(description="Panes Mem0 Sidecar")
    parser.add_argument("--port", type=int, default=11435)
    parser.add_argument("--data-dir", default=os.path.expanduser("~/Library/Application Support/dev.panes/mem0"))
    parser.add_argument("--llm-provider", default="aws_bedrock")
    parser.add_argument("--llm-model", default="us.anthropic.claude-sonnet-4-6")
    args = parser.parse_args()

    app.state.data_dir = args.data_dir
    app.state.llm_provider = args.llm_provider
    app.state.llm_model = args.llm_model

    import uvicorn
    uvicorn.run(app, host="127.0.0.1", port=args.port, log_level="info")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, stream=sys.stderr)
    main()
