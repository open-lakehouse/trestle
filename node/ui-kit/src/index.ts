// Public surface of the shared UI kit: the shadcn/Radix primitives, the `cn`
// class-merge helper, and the theme context. Feature packages
// (@open-lakehouse/env-editor) and the app depend on this package rather than
// each carrying its own copy of the primitives. Distribute visuals from one
// place; the token contract lives in ./DESIGN.md.

export { Badge, type BadgeProps, badgeVariants } from "./badge";
export { Button, type ButtonProps, buttonVariants } from "./button";
export {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "./card";
export {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogOverlay,
  DialogPortal,
  DialogTitle,
  DialogTrigger,
} from "./dialog";
export { Input } from "./input";
export { Label } from "./label";
export {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "./select";
export { Separator } from "./separator";
export {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetOverlay,
  SheetPortal,
  SheetTitle,
  SheetTrigger,
} from "./sheet";
export { Toaster } from "./sonner";
export { Switch, type SwitchProps } from "./switch";
export { ThemeProvider, useTheme } from "./ThemeProvider";
export { Tabs, TabsContent, TabsList, TabsTrigger } from "./tabs";
export {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "./tooltip";
export { cn } from "./utils";
