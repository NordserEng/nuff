from sqlalchemy import ForeignKey
from sqlalchemy.orm import Mapped, mapped_column

from app.platform.models import Base


class Charge(Base):
    __tablename__ = "charges"
    company_id: Mapped[int] = mapped_column(ForeignKey("companies.id"))  # NUF004
    receipt_id: Mapped[int] = mapped_column(ForeignKey("receipts.id"))
    agreement_id: Mapped[int] = mapped_column(ForeignKey("agreements.id"))  # NUF004 unlisted


class Receipt(Base):
    __tablename__ = "receipts"
    charge_id: Mapped[int] = mapped_column(ForeignKey(column="charges.id"))
