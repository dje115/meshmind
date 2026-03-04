/**
 * Minimal central state for the app. Single source of truth for currentPage,
 * statusData, pollInterval, and chatState so they are not scattered globals.
 */
const state = {
  currentPage: 'dashboard',
  statusData: null,
  pollInterval: null,
  chatState: {
    conversationId: null,
    conversations: [],
    sending: false,
    searchQuery: '',
  },
};

export const store = {
  getCurrentPage: () => state.currentPage,
  setCurrentPage: (page) => { state.currentPage = page; },

  getStatusData: () => state.statusData,
  setStatusData: (data) => { state.statusData = data; },

  getPollInterval: () => state.pollInterval,
  setPollInterval: (id) => { state.pollInterval = id; },

  getChatState: () => state.chatState,
};
