'use client';

import React from 'react';

interface MainContentProps {
  children: React.ReactNode;
}

const MainContent: React.FC<MainContentProps> = ({ children }) => (
  <main className="relative flex min-w-0 flex-1 flex-col overflow-hidden">{children}</main>
);

export default MainContent;
