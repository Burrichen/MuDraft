import { useId, type ButtonHTMLAttributes, type ReactNode } from "react";

type Variant = "primary" | "secondary" | "ghost" | "danger";

interface Props extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "disabled"> {
  variant?: Variant;
  children: ReactNode;
  /**
   * When set, the button is unavailable and this reason is shown as visible text next to
   * it and linked with aria-describedby — never hover-only. The button stays focusable
   * (aria-disabled) so keyboard and screen-reader users can discover why.
   */
  disabledReason?: string | undefined;
}

export function Button({
  variant = "secondary",
  disabledReason,
  className,
  onClick,
  children,
  type = "button",
  ...rest
}: Props) {
  const reasonId = useId();
  const unavailable = disabledReason !== undefined;
  const button = (
    <button
      {...rest}
      type={type}
      className={["button", `button-${variant}`, className].filter(Boolean).join(" ")}
      aria-disabled={unavailable || undefined}
      aria-describedby={
        unavailable
          ? [rest["aria-describedby"], reasonId].filter(Boolean).join(" ")
          : rest["aria-describedby"]
      }
      onClick={(e) => {
        if (unavailable) {
          e.preventDefault();
          return;
        }
        onClick?.(e);
      }}
    >
      {children}
    </button>
  );
  // One stable structure whether or not a reason is shown, so a change in availability
  // never remounts the button (which would drop keyboard focus).
  return (
    <span className="button-with-reason">
      {button}
      {unavailable && (
        <span id={reasonId} className="disabled-reason">
          {disabledReason}
        </span>
      )}
    </span>
  );
}
