import { QRCode } from "react-qr-code";
import { cn } from "@/lib/utils";

const TOTP_QR_CODE_SIZE = 256;

type TotpQrCodeProps = {
  value: string;
  id?: string;
  className?: string;
  ariaLabel?: string;
};

export function TotpQrCode({
  value,
  id,
  className,
  ariaLabel = "TOTP setup QR code",
}: TotpQrCodeProps) {
  return (
    <div
      id={id}
      className={cn("inline-flex rounded-md bg-white p-6 shadow-sm", className)}
    >
      <QRCode
        aria-label={ariaLabel}
        bgColor="#FFFFFF"
        className="block h-auto max-w-full"
        fgColor="#000000"
        level="L"
        shapeRendering="crispEdges"
        size={TOTP_QR_CODE_SIZE}
        value={value}
      />
    </div>
  );
}
