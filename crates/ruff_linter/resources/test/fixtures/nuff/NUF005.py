import asyncio
from typing import Annotated

from fastapi import Depends
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from app.accounts.services import brreg
from app.billing.services import vipps


async def read_then_call(session: AsyncSession, company_id: int) -> None:
    company = await session.get(Company, company_id)
    await vipps.stop_agreement(company.agreement_id)  # NUF005


async def thread_hop_still_counts(session: AsyncSession, org: str) -> None:
    await session.execute(select(Company))
    await asyncio.to_thread(brreg.fetch_company, org)  # NUF005


async def annotated(session: Annotated[AsyncSession, Depends()], company_id: int) -> None:
    session.add(Company())
    await vipps.create_charge("a")  # NUF005


async def begin_block(session: AsyncSession) -> None:
    async with session.begin():
        await vipps.list_charges("a")  # NUF005
    await vipps.list_charges("a")


async def commit_then_call(session: AsyncSession, company_id: int) -> None:
    company = await session.get(Company, company_id)
    agreement_id = company.agreement_id
    await session.commit()
    await vipps.stop_agreement(agreement_id)
    locked = await session.get(Company, company_id, with_for_update=True)
    await session.commit()


async def call_before_reading(session: AsyncSession, org: str) -> None:
    view = await asyncio.to_thread(brreg.fetch_company, org)
    session.add(Company(name=view["name"]))
    await session.commit()


async def rollback_then_call(session: AsyncSession) -> None:
    await session.execute(select(Company))
    await session.rollback()
    await vipps.agreement_status("a")


async def no_session(company_id: int) -> None:
    await vipps.stop_agreement("a")


async def nested_function_is_its_own_scope(session: AsyncSession) -> None:
    await session.execute(select(Company))

    async def later() -> None:
        await vipps.stop_agreement("a")

    await session.commit()


async def helper_reads_through_session(session: AsyncSession, company_id: int) -> None:
    company = await billable_company(session, company_id)
    await vipps.stop_agreement(company.agreement_id)  # NUF005


async def helper_then_commit(session: AsyncSession, company_id: int) -> None:
    company = await billable_company(session, company_id)
    agreement_id = company.agreement_id
    await session.commit()
    await vipps.stop_agreement(agreement_id)


async def branch_that_returns_keeps_its_state(session: AsyncSession, invite: str, org: str) -> None:
    if invite:
        await validate_invite_row(session, invite)
        return
    if org:
        await asyncio.to_thread(brreg.fetch_company, org)


async def branch_that_falls_through_leaks(session: AsyncSession, invite: str, org: str) -> None:
    if invite:
        await validate_invite_row(session, invite)
    else:
        pass
    await asyncio.to_thread(brreg.fetch_company, org)  # NUF005


async def every_branch_commits(session: AsyncSession, flag: bool) -> None:
    await session.execute(select(Company))
    if flag:
        await session.commit()
    else:
        await session.rollback()
    await vipps.agreement_status("a")
