from typing import Any, Dict, Optional

from fastapi import APIRouter
from pydantic import BaseModel

router = APIRouter()


class Health(BaseModel):
    status: str


@router.get("/dict")
async def bare_dict() -> dict:  # NUF002
    return {}


@router.get("/dict-any")
async def parameterised_dict() -> dict[str, Any]:  # NUF002
    return {}


@router.post("/typing-dict")
async def typing_dict() -> Dict[str, str]:  # NUF002
    return {}


@router.get("/optional")
async def optional_dict() -> Optional[dict[str, str]]:  # NUF002
    return None


@router.get("/union")
async def union_dict() -> dict[str, str] | None:  # NUF002
    return None


@router.get("/health")
async def health() -> Health:
    return Health(status="ok")


async def helper() -> dict[str, Any]:
    return {}
