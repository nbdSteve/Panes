"""Panes mem0 server — thin wrapper that configures mem0 from environment and serves a REST API.

Start with: python mem0_server.py [--port PORT]

Environment variables:
  PANES_MEM0_PORT         — Port to listen on (default: 8019)
  CLAUDE_CODE_USE_BEDROCK — When set, use AWS Bedrock for LLM and embeddings
  AWS_PROFILE             — AWS profile for Bedrock auth
  AWS_REGION              — AWS region for Bedrock (default: us-east-1)
"""

import os
import sys
import argparse

from mem0 import Memory
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel


def build_config():
    config = {}

    if os.environ.get("CLAUDE_CODE_USE_BEDROCK"):
        region = os.environ.get("AWS_REGION", "us-east-1")
        os.environ.setdefault("AWS_REGION", region)

        config["llm"] = {
            "provider": "aws_bedrock",
            "config": {
                "model": "us.anthropic.claude-haiku-4-5-20251001-v1:0",
                "temperature": 0.1,
                "max_tokens": 1500,
            },
        }
        config["embedder"] = {
            "provider": "aws_bedrock",
            "config": {
                "model": "amazon.titan-embed-text-v2:0",
            },
        }
        config["vector_store"] = {
            "provider": "qdrant",
            "config": {
                "embedding_model_dims": 1024,
            },
        }

    return config


config = build_config()
memory = Memory.from_config(config) if config else Memory()
app = FastAPI(redirect_slashes=False)


class AddRequest(BaseModel):
    messages: list[dict]
    user_id: str | None = None
    agent_id: str | None = None
    run_id: str | None = None
    metadata: dict | None = None


class SearchRequest(BaseModel):
    query: str
    user_id: str | None = None
    agent_id: str | None = None
    run_id: str | None = None
    top_k: int = 10


class UpdateRequest(BaseModel):
    text: str
    metadata: dict | None = None


@app.post("/memories")
async def add_memory(req: AddRequest):
    kwargs = {"messages": req.messages}
    if req.user_id:
        kwargs["user_id"] = req.user_id
    if req.agent_id:
        kwargs["agent_id"] = req.agent_id
    if req.run_id:
        kwargs["run_id"] = req.run_id
    if req.metadata:
        kwargs["metadata"] = req.metadata
    result = memory.add(**kwargs)
    return result


@app.get("/health")
async def health():
    return {"status": "ok"}


@app.get("/memories")
async def get_memories(user_id: str | None = None, agent_id: str | None = None, run_id: str | None = None):
    filters = {}
    if user_id:
        filters["user_id"] = user_id
    if agent_id:
        filters["agent_id"] = agent_id
    if run_id:
        filters["run_id"] = run_id
    if not filters:
        return {"results": []}
    result = memory.get_all(filters=filters)
    return result


@app.get("/memories/{memory_id}")
async def get_memory(memory_id: str):
    result = memory.get(memory_id)
    if not result:
        raise HTTPException(status_code=404, detail="Memory not found")
    return result


@app.put("/memories/{memory_id}")
async def update_memory(memory_id: str, req: UpdateRequest):
    result = memory.update(memory_id, data=req.text)
    return result


@app.delete("/memories/{memory_id}")
async def delete_memory(memory_id: str):
    memory.delete(memory_id)
    return {"message": "Memory deleted successfully"}


@app.post("/search")
async def search_memories(req: SearchRequest):
    filters = {}
    if req.user_id:
        filters["user_id"] = req.user_id
    if req.agent_id:
        filters["agent_id"] = req.agent_id
    if req.run_id:
        filters["run_id"] = req.run_id
    result = memory.search(req.query, top_k=req.top_k, filters=filters if filters else None)
    return result


if __name__ == "__main__":
    import uvicorn

    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=int(os.environ.get("PANES_MEM0_PORT", "8019")))
    parser.add_argument("--host", default="127.0.0.1")
    args = parser.parse_args()

    uvicorn.run(app, host=args.host, port=args.port)
