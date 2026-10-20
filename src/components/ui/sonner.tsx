import { Toaster as Sonner, type ToasterProps } from "sonner";

const DEFAULT_OFFSET = {
  top: "3rem",
  right: "1.5rem",
  bottom: "1.5rem",
  left: "1.5rem",
} satisfies ToasterProps["offset"];

const DEFAULT_MOBILE_OFFSET = {
  top: "3rem",
  right: "1rem",
  bottom: "1rem",
  left: "1rem",
} satisfies ToasterProps["mobileOffset"];

const Toaster = ({ ...props }: ToasterProps) => {
  return (
    <Sonner
      theme="dark"
      position="top-right"
      closeButton
      offset={DEFAULT_OFFSET}
      mobileOffset={DEFAULT_MOBILE_OFFSET}
      className="toaster group"
      richColors
      style={
        {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border)",
          "--border-radius": "var(--radius)",
        } as React.CSSProperties
      }
      {...props}
    />
  );
};

export { Toaster };
