import { redirect } from "react-router";

export function loader() {
  return redirect("/form");
}

export default function DeskRedirect() {
  return null;
}
