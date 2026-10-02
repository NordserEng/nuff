import asyncio
import xml.etree.ElementTree as ET

from app.accounts.services import brreg
from app.platform.email import send_email


async def direct(email: str) -> None:
    send_email(email, "Welcome")  # NUF001


async def through_module(org: str) -> None:
    brreg.fetch_company(org)  # NUF001


async def standard_library(content: bytes) -> None:
    ET.fromstring(content)  # NUF001


async def through_thread(email: str, org: str, content: bytes) -> None:
    await asyncio.to_thread(send_email, email, "Welcome")
    await asyncio.to_thread(brreg.fetch_company, org)
    await asyncio.to_thread(lambda: ET.fromstring(content))


def synchronous(email: str) -> None:
    send_email(email, "Welcome")


async def nested_synchronous(email: str) -> None:
    def helper() -> None:
        send_email(email, "Welcome")

    await asyncio.to_thread(helper)


async def unlisted(org: str) -> None:
    brreg.search_companies(org)
