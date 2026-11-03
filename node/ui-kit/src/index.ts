// Public surface of the shared UI kit: the shadcn/Radix primitives, the `cn`
// class-merge helper, and the theme context. Every Open Lakehouse web UI
// (trestle's env-editor, mangrove's Unity Catalog UI, headwaters' lineage UI)
// depends on this package rather than carrying its own copy of the primitives.
// The token contract lives in ./DESIGN.md.

export { Badge, type BadgeProps, badgeVariants } from "./badge.js";
export { Button, type ButtonProps, buttonVariants } from "./button.js";
export {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "./card.js";
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
} from "./dialog.js";
export {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPortal,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "./dropdown-menu.js";
export { Input } from "./input.js";
export { Label } from "./label.js";
export {
  ResizeHandle,
  type ResizeHandleProps,
  usePaneSize,
} from "./resizable.js";
export {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "./select.js";
export { Separator } from "./separator.js";
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
} from "./sheet.js";
export { Toaster } from "./sonner.js";
export { Switch, type SwitchProps } from "./switch.js";
export { ThemeProvider, useTheme } from "./ThemeProvider.js";
export { Tabs, TabsContent, TabsList, TabsTrigger } from "./tabs.js";
export { Textarea } from "./textarea.js";
export {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "./tooltip.js";
export { cn } from "./utils.js";
