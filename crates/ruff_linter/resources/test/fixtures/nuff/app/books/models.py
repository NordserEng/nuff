import sqlalchemy as sa
from sqlalchemy.orm import Mapped, mapped_column

import app.accounts.models  # noqa: F401
from app.billing import models as billing_models  # noqa: F401
from app.platform.models import Base


class Posting(Base):
    __tablename__ = "postings"
    company_id: Mapped[int] = mapped_column(sa.ForeignKey("companies.id"))
    charge_id: Mapped[int] = mapped_column(sa.ForeignKey("public.charges.id"))
    lock_id: Mapped[int] = mapped_column(sa.ForeignKey("period_locks.id"))  # NUF004 unlisted
