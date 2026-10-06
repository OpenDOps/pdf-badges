import { useEffect } from "react";
import { Outlet, redirect, useLocation, useNavigate } from "react-router";

import { FormClient, isUnauthorized, setUnauthorized } from "../form/client";
import { DeskHeader, DeskNav, isDeskNav } from "../shell/header";
import { NetworkCorner } from "../shell/network";

export async function loader({ request }: { request: Request }) {
  try {
    const sync = await FormClient.fromRequest(request).sync();
    return {
      waiting: sync.waiting,
      synced: sync.synced,
      addresses: sync.addresses ?? [],
      admin: sync.is_admin === true,
      origin: new URL(request.url).origin,
    };
  } catch (error) {
    if (isUnauthorized(error)) return redirect("/key");
    throw error;
  }
}

export default function OperatorLayout() {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  setUnauthorized(() => {
    navigate("/key", { replace: true });
  });
  useEffect(() => {
    return () => setUnauthorized(undefined);
  }, []);
  const desk = isDeskNav(pathname);
  return (
    <>
      {desk ? <DeskNav /> : <NetworkCorner />}
      {desk ? null : <DeskHeader />}
      <Outlet />
    </>
  );
}
