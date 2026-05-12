import {
  JOB_RUN_FIELDS,
  SUBTITLE_PROVIDER_CONFIG_FIELDS,
  SUBTITLE_SETTINGS_FIELDS,
  TITLE_CORE_FIELDS,
} from "./queries";

export const loginMutation = `mutation Login($input: LoginInput!) {
  login(input: $input) {
    token
    user {
      id
      username
      appPermissions
      libraryPermissions {
        libraryId
        permissions
      }
    }
    expiresAt
  }
}`;

export const createUserMutation = `mutation CreateUser($input: CreateUserInput!) {
  createUser(input: $input) {
    id
    username
    appPermissions
    libraryPermissions {
      libraryId
      permissions
    }
  }
}`;

export const setUserPasswordMutation = `mutation SetUserPassword($input: SetUserPasswordInput!) {
  setUserPassword(input: $input) {
    id
    username
    appPermissions
    libraryPermissions {
      libraryId
      permissions
    }
  }
}`;

export const setUserAppPermissionsMutation = `mutation SetUserAppPermissions($input: SetUserAppPermissionsInput!) {
  setUserAppPermissions(input: $input) {
    id
    username
    appPermissions
    libraryPermissions {
      libraryId
      permissions
    }
  }
}`;

export const setUserLibraryPermissionsMutation = `mutation SetUserLibraryPermissions($input: SetUserLibraryPermissionsInput!) {
  setUserLibraryPermissions(input: $input) {
    id
    username
    appPermissions
    libraryPermissions {
      libraryId
      permissions
    }
  }
}`;

export const deleteUserMutation = `mutation DeleteUser($input: DeleteUserInput!) {
  deleteUser(input: $input)
}`;

export const deleteTitleMutation = `mutation DeleteTitle($input: DeleteTitleInput!) {
  deleteTitle(input: $input)
}`;

export const createIndexerMutation = `mutation CreateIndexer($input: CreateIndexerConfigInput!) {
  createIndexerConfig(input: $input) {
    id
    name
    providerType
    baseUrl
    hasApiKey
    storedSecretKeys
    rateLimitSeconds
    rateLimitBurst
    disabledUntil
    isEnabled
    enableInteractiveSearch
    enableAutoSearch
    lastHealthStatus
    lastErrorAt
    configJson
    createdAt
    updatedAt
  }
}`;

export const updateIndexerMutation = `mutation UpdateIndexer($input: UpdateIndexerConfigInput!) {
  updateIndexerConfig(input: $input) {
    id
    name
    providerType
    baseUrl
    hasApiKey
    storedSecretKeys
    rateLimitSeconds
    rateLimitBurst
    disabledUntil
    isEnabled
    enableInteractiveSearch
    enableAutoSearch
    lastHealthStatus
    lastErrorAt
    configJson
    createdAt
    updatedAt
  }
}`;

export const deleteIndexerMutation = `mutation DeleteIndexer($input: DeleteIndexerConfigInput!) {
  deleteIndexerConfig(input: $input)
}`;

export const testIndexerConnectionMutation = `mutation TestIndexerConnection($input: TestIndexerConnectionInput!) {
  testIndexerConnection(input: $input)
}`;

export const createDownloadClientMutation = `mutation CreateDownloadClient($input: CreateDownloadClientConfigInput!) {
  createDownloadClientConfig(input: $input) {
    id
    name
    clientType
    baseUrl
    configJson
    isEnabled
    status
    lastError
    lastSeenAt
    createdAt
    updatedAt
  }
}`;

export const updateDownloadClientMutation = `mutation UpdateDownloadClient($input: UpdateDownloadClientConfigInput!) {
  updateDownloadClientConfig(input: $input) {
    id
    name
    clientType
    baseUrl
    configJson
    isEnabled
    status
    lastError
    lastSeenAt
    createdAt
    updatedAt
  }
}`;

export const testDownloadClientConnectionMutation = `mutation TestDownloadClientConnection($input: TestDownloadClientConnectionInput!) {
  testDownloadClientConnection(input: $input)
}`;

export const deleteDownloadClientMutation = `mutation DeleteDownloadClient($input: DeleteDownloadClientConfigInput!) {
  deleteDownloadClientConfig(input: $input)
}`;

export const reorderDownloadClientsMutation = `mutation ReorderDownloadClients($input: ReorderDownloadClientConfigsInput!) {
  reorderDownloadClientConfigs(input: $input)
}`;

export const addTitleMutation = `mutation AddTitle($input: AddTitleInput!) {
  addTitle(input: $input) {
    title {${TITLE_CORE_FIELDS}
    }
    metadataHydrationState
    reusedExistingTitle
    reusedQueuedDownload
    downloadJobId
    queuedDownload {
      jobId
      titleId
      titleName
      sourceTitle
      sourceKind
    }
  }
}`;

export const addTitleAndQueueMutation = `mutation AddTitleAndQueue($input: AddTitleInput!) {
  addTitleAndQueueDownload(input: $input) {
    title {${TITLE_CORE_FIELDS}
    }
    metadataHydrationState
    reusedExistingTitle
    reusedQueuedDownload
    downloadJobId
    queuedDownload {
      jobId
      titleId
      titleName
      sourceTitle
      sourceKind
    }
  }
}`;

export const deleteMediaFileMutation = `mutation DeleteMediaFile($input: DeleteMediaFileInput!) {
  deleteMediaFile(input: $input)
}`;

export const scanLibraryMutation = `mutation ScanLibrary($libraryId: String!) {
  scanLibrary(libraryId: $libraryId) {
    sessionId
    facet
    mode
    status
    startedAt
    updatedAt
  }
}`;

const LIBRARY_FIELDS = `
    id
    facet
    name
    slug
    isDefault
    roots {
      id
      path
      isDefault
    }`;

export const createLibraryMutation = `mutation CreateLibrary($input: CreateLibraryInput!) {
  createLibrary(input: $input) {${LIBRARY_FIELDS}
  }
}`;

export const updateLibraryMutation = `mutation UpdateLibrary($input: UpdateLibraryInput!) {
  updateLibrary(input: $input) {${LIBRARY_FIELDS}
  }
}`;

export const deleteLibraryMutation = `mutation DeleteLibrary($input: DeleteLibraryInput!) {
  deleteLibrary(input: $input)
}`;

export const cancelLibraryScanMutation = `mutation CancelLibraryScan($input: CancelLibraryScanInput!) {
  cancelLibraryScan(input: $input) {
    sessionId
    accepted
  }
}`;

export const scanTitleLibraryMutation = `mutation ScanTitleLibrary($input: TitleIdInput!) {
  scanTitleLibrary(input: $input) {
    scanned
    matched
    imported
    skipped
    unmatched
  }
}`;

export const resolvePendingImportMutation = `mutation ResolvePendingImport($input: ResolvePendingImportInput!) {
  resolvePendingImport(input: $input) {
    created
    libraryScan {
      scanned
      matched
      imported
      skipped
      unmatched
    }
    title {
      id
      name
      facet
      monitored
    }
  }
}`;

export const bindPendingImportMutation = `mutation BindPendingImport($input: BindPendingImportInput!) {
  bindPendingImport(input: $input) {
    created
    libraryScan {
      scanned
      matched
      imported
      skipped
      unmatched
    }
    title {
      id
      name
      facet
      monitored
    }
  }
}`;

export const ignorePendingImportMutation = `mutation IgnorePendingImport($input: IgnorePendingImportInput!) {
  ignorePendingImport(input: $input) {
    id
    status
  }
}`;

export const triggerJobMutation = `mutation TriggerJob($jobKey: JobKeyValue!) {
  triggerJob(jobKey: $jobKey) {
${JOB_RUN_FIELDS}
  }
}`;

export const applyMediaRenameMutation = `mutation ApplyMediaRename($input: MediaRenameApplyInput!) {
  applyMediaRename(input: $input) {
    planFingerprint
    total
    applied
    skipped
    failed
    items {
      collectionId
      currentPath
      proposedPath
      finalPath
      writeAction
      status
      reasonCode
      errorMessage
    }
  }
}`;

export const applyMediaRenameBulkMutation = `mutation ApplyMediaRenameBulk($input: MediaRenameBulkApplyInput!) {
  applyMediaRenameBulk(input: $input) {
    planFingerprint
    total
    applied
    skipped
    failed
    items {
      collectionId
      currentPath
      proposedPath
      finalPath
      writeAction
      status
      reasonCode
      errorMessage
    }
  }
}`;

export const updateSubtitleSettingsMutation = `mutation UpdateSubtitleSettings($input: UpdateSubtitleSettingsInput!) {
  updateSubtitleSettings(input: $input) {${SUBTITLE_SETTINGS_FIELDS}
  }
}`;

export const createSubtitleProviderConfigMutation = `mutation CreateSubtitleProviderConfig($input: CreateSubtitleProviderConfigInput!) {
  createSubtitleProviderConfig(input: $input) {${SUBTITLE_PROVIDER_CONFIG_FIELDS}
  }
}`;

export const updateSubtitleProviderConfigMutation = `mutation UpdateSubtitleProviderConfig($input: UpdateSubtitleProviderConfigInput!) {
  updateSubtitleProviderConfig(input: $input) {${SUBTITLE_PROVIDER_CONFIG_FIELDS}
  }
}`;

export const deleteSubtitleProviderConfigMutation = `mutation DeleteSubtitleProviderConfig($input: DeleteSubtitleProviderConfigInput!) {
  deleteSubtitleProviderConfig(input: $input)
}`;

export const testSubtitleProviderConnectionMutation = `mutation TestSubtitleProviderConnection($input: TestSubtitleProviderConnectionInput!) {
  testSubtitleProviderConnection(input: $input) {
    status
    message
    retryAfterSeconds
  }
}`;

export const updateAcquisitionSettingsMutation = `mutation UpdateAcquisitionSettings($input: UpdateAcquisitionSettingsInput!) {
  updateAcquisitionSettings(input: $input) {
    enabled
    upgradeCooldownHours
    sameTierMinDelta
    crossTierMinDelta
    forcedUpgradeDeltaBypass
    pollIntervalSeconds
    syncIntervalSeconds
    batchSize
  }
}`;

export const updateGeneralSettingsMutation = `mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
  updateGeneralSettings(input: $input) {
    keepHistoryForever
    historyRetentionDays
  }
}`;

export const updateSecuritySettingsMutation = `mutation UpdateSecuritySettings($input: UpdateSecuritySettingsInput!) {
  updateSecuritySettings(input: $input) {
    formLoginEnabled
    skipLoginForLocalIps
    effectiveFormLoginEnabled
    envOverrideActive
    envOverrideDescription
  }
}`;

export const upsertDelayProfileMutation = `mutation UpsertDelayProfile($input: DelayProfileInput!) {
  upsertDelayProfile(input: $input) {
    id
    name
    usenetDelayMinutes
    torrentDelayMinutes
    preferredProtocol
    minAgeMinutes
    bypassScoreThreshold
    appliesToFacets
    tags
    priority
    enabled
  }
}`;

export const deleteDelayProfileMutation = `mutation DeleteDelayProfile($input: DeleteDelayProfileInput!) {
  deleteDelayProfile(input: $input) {
    id
  }
}`;

const qualityProfileCriteriaFields = `
      qualityTiers
      archivalQuality
      allowUnknownQuality
      sourceAllowlist
      sourceBlocklist
      videoCodecAllowlist
      videoCodecBlocklist
      audioCodecAllowlist
      audioCodecBlocklist
      dolbyVisionAllowed
      detectedHdrAllowed
      preferRemux
      allowBdDisk
      allowUpgrades
      scoringOverrides {
        allowX265Non4K
        blockDvWithoutFallback
        preferCompactEncodes
        preferLosslessAudio
        blockUpscaled
      }
      cutoffTier
      minScoreToGrab`;

const qualityProfileSettingsFieldSelection = `
    globalProfileId
    globalScoringPersona
    profiles {
      id
      name
      criteria {${qualityProfileCriteriaFields}
      }
    }
    categorySelections {
      scope
      overrideProfileId
      effectiveProfileId
      inheritsGlobal
    }
    categoryPersonaSelections {
      scope
      overridePersona
      effectivePersona
      inheritsGlobal
    }`;

const downloadClientRoutingFieldSelection = `
    clientId
    enabled
    category
    recentQueuePriority
    olderQueuePriority
    removeCompleted
    removeFailed`;

const indexerRoutingFieldSelection = `
    indexerId
    enabled
    categories
    priority`;

const mediaSettingsFieldSelection = `
    scope
    libraryPath
    rootFolders {
      path
      isDefault
    }
    requiredAudioLanguages
    renameTemplate
    renameCollisionPolicy
    renameMissingMetadataPolicy
    fillerPolicy
    recapPolicy
    monitorSpecials
    interSeasonMovies
    monitorFillerMovies
    nfoWriteOnImport
    plexmatchWriteOnImport`;

const libraryPathsFieldSelection = `
    moviePath
    seriesPath
    animePath`;

const serviceSettingsFieldSelection = `
    tlsCertPath
    tlsKeyPath`;

export const saveQualityProfileSettingsMutation = `mutation SaveQualityProfileSettings($input: SaveQualityProfileSettingsInput!) {
  saveQualityProfileSettings(input: $input) {${qualityProfileSettingsFieldSelection}
  }
}`;

export const deleteQualityProfileMutation = `mutation DeleteQualityProfile($input: DeleteQualityProfileInput!) {
  deleteQualityProfile(input: $input) {
${qualityProfileSettingsFieldSelection}
  }
}`;

export const updateDownloadClientRoutingMutation = `mutation UpdateDownloadClientRouting($input: UpdateDownloadClientRoutingInput!) {
  updateDownloadClientRouting(input: $input) {${downloadClientRoutingFieldSelection}
  }
}`;

export const updateIndexerRoutingMutation = `mutation UpdateIndexerRouting($input: UpdateIndexerRoutingInput!) {
  updateIndexerRouting(input: $input) {${indexerRoutingFieldSelection}
  }
}`;

export const updateMediaSettingsMutation = `mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
  updateMediaSettings(input: $input) {${mediaSettingsFieldSelection}
  }
}`;

export const updateLibraryPathsMutation = `mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
  updateLibraryPaths(input: $input) {${libraryPathsFieldSelection}
  }
}`;

export const updateServiceSettingsMutation = `mutation UpdateServiceSettings($input: UpdateServiceSettingsInput!) {
  updateServiceSettings(input: $input) {${serviceSettingsFieldSelection}
  }
}`;

export const queueExistingMutation = `mutation QueueExisting($input: QueueDownloadInput!) {
  queueExistingTitleDownload(input: $input) {
    status
    jobId
    titleId
    titleName
    sourceTitle
    sourceKind
    conflict {
      titleId
      titleName
      downloadClientId
      downloadClientType
      downloadClientItemId
      sourceTitle
      sourceKind
      state
      replaceable
      scope {
        kind
        episodeId
        episodeIds
        collectionId
      }
    }
  }
}`;

export const triggerTitleMismatchRecoverySearchMutation = `mutation TriggerTitleMismatchRecoverySearch($input: TitleIdInput!) {
  triggerTitleMismatchRecoverySearch(input: $input)
}`;

export const queueBestReleaseMutation = `mutation QueueBestRelease($input: QueueBestReleaseInput!) {
  queueBestRelease(input: $input) {
    status
    jobId
    titleId
    titleName
    sourceTitle
    sourceKind
    conflict {
      titleId
      titleName
      downloadClientId
      downloadClientType
      downloadClientItemId
      sourceTitle
      sourceKind
      state
      replaceable
      scope {
        kind
        episodeId
        episodeIds
        collectionId
      }
    }
  }
}`;

export const queueManualImportMutation = `mutation QueueManualImport($input: QueueManualImportInput!) {
  queueManualImport(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    importId
    removed
    queueItem {
      id
      titleId
      titleName
      clientId
      clientType
      downloadClientItemId
      state
      importStatus
      importErrorCode
      importErrorMessage
      trackedState
    }
  }
}`;

export const pauseDownloadMutation = `mutation PauseDownload($input: PauseDownloadInput!) {
  pauseDownload(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    removed
    queueItem {
      id
      clientId
      clientType
      downloadClientItemId
      state
    }
  }
}`;

export const resumeDownloadMutation = `mutation ResumeDownload($input: ResumeDownloadInput!) {
  resumeDownload(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    removed
    queueItem {
      id
      clientId
      clientType
      downloadClientItemId
      state
    }
  }
}`;

export const deleteDownloadMutation = `mutation DeleteDownload($input: DeleteDownloadInput!) {
  deleteDownload(input: $input) {
    kind
    downloadClientItemId
    clientId
    commandId
    removed
    clientType
    queueItem {
      id
      clientId
      clientType
      downloadClientItemId
      state
      deleteStatus
      deleteErrorMessage
    }
  }
}`;

export function buildIgnoreTrackedDownloadBatchMutation(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$input${index}: IgnoreTrackedDownloadInput!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) => `item${index}: ignoreTrackedDownload(input: $input${index}) { kind }`,
  ).join("\n");

  return `mutation IgnoreTrackedDownloads(${variables}) {
${fields}
}`;
}

export function buildDeleteDownloadBatchMutation(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$input${index}: DeleteDownloadInput!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) => `item${index}: deleteDownload(input: $input${index}) { kind removed commandId }`,
  ).join("\n");

  return `mutation DeleteDownloads(${variables}) {
${fields}
}`;
}

export const setCollectionMonitoredMutation = `mutation SetCollectionMonitored($input: SetCollectionMonitoredInput!) {
  setCollectionMonitored(input: $input) {
    id
    monitored
    episodes {
      id
      titleId
      collectionId
      episodeType
      episodeNumber
      seasonNumber
      episodeLabel
      title
      overview
      airDate
      durationSeconds
      hasMultiAudio
      hasSubtitle
      isFiller
      absoluteNumber
      monitored
      createdAt
    }
  }
}`;

export const setEpisodeMonitoredMutation = `mutation SetEpisodeMonitored($input: SetEpisodeMonitoredInput!) {
  setEpisodeMonitored(input: $input) { id monitored }
}`;

export const setTitleMonitoredMutation = `mutation SetTitleMonitored($input: SetTitleMonitoredInput!) {
  setTitleMonitored(input: $input) { id monitored }
}`;

export const updateTitleMutation = `mutation UpdateTitle($input: UpdateTitleInput!) {
  updateTitle(input: $input) {
    id
    name
    facet
    tags
    monitored
    qualityProfileId
    rootFolderPath
    monitorType
    useSeasonFolders
    monitorSpecials
    interSeasonMovies
    fillerPolicy
    recapPolicy
  }
}`;

export function buildSetTitleMonitoredBatchMutation(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$input${index}: SetTitleMonitoredInput!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) =>
      `item${index}: setTitleMonitored(input: $input${index}) { id monitored }`,
  ).join("\n");

  return `mutation SetTitleMonitoredBatch(${variables}) {
${fields}
}`;
}

export function buildUpdateTitleBatchMutation(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$input${index}: UpdateTitleInput!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) => `item${index}: updateTitle(input: $input${index}) { id }`,
  ).join("\n");

  return `mutation UpdateTitleBatch(${variables}) {
${fields}
}`;
}

export function buildDeleteTitleBatchMutation(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$input${index}: DeleteTitleInput!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) => `item${index}: deleteTitle(input: $input${index})`,
  ).join("\n");

  return `mutation DeleteTitleBatch(${variables}) {
${fields}
}`;
}

export const fixTitleMatchMutation = `mutation FixTitleMatch($input: FixTitleMatchInput!) {
  fixTitleMatch(input: $input) {
    hydrated
    warnings
    libraryScan {
      scanned
      matched
      imported
      skipped
      unmatched
    }
    title {
      id
      name
      facet
      externalIds {
        source
        value
      }
      imdbId
      slug
      metadataFetchedAt
    }
  }
}`;

const wantedSearchPayloadSelection = `
    queuedCount
    skippedInProgressCount
    conflict {
      titleId
      titleName
      downloadClientId
      downloadClientType
      downloadClientItemId
      sourceTitle
      sourceKind
      state
      replaceable
      scope {
        kind
        episodeId
        episodeIds
        collectionId
      }
    }`;

export const triggerWantedSearchMutation = `mutation TriggerWantedSearch($input: TriggerWantedSearchInput!) {
  triggerWantedSearch(input: $input) {${wantedSearchPayloadSelection}
  }
}`;

export const triggerTitleWantedSearchMutation = `mutation TriggerTitleWantedSearch($input: TriggerTitleWantedSearchInput!) {
  triggerTitleWantedSearch(input: $input) {${wantedSearchPayloadSelection}
  }
}`;

export const triggerSeasonWantedSearchMutation = `mutation TriggerSeasonWantedSearch($input: TriggerSeasonWantedSearchInput!) {
  triggerSeasonWantedSearch(input: $input) {${wantedSearchPayloadSelection}
  }
}`;

export const pauseWantedItemMutation = `mutation PauseWantedItem($input: WantedItemIdInput!) {
  pauseWantedItem(input: $input)
}`;

export const resumeWantedItemMutation = `mutation ResumeWantedItem($input: WantedItemIdInput!) {
  resumeWantedItem(input: $input)
}`;

export const resetWantedItemMutation = `mutation ResetWantedItem($input: WantedItemIdInput!) {
  resetWantedItem(input: $input)
}`;

// ── RSS Sync ─────────────────────────────────────────────────────────────

export const triggerRssSyncMutation = `mutation TriggerRssSync {
  triggerRssSync {
    releasesFetched
    releasesMatched
    releasesGrabbed
    releasesHeld
  }
}`;

// ── Pending Releases ─────────────────────────────────────────────────────

export const forceGrabPendingReleaseMutation = `mutation ForceGrabPendingRelease($input: PendingReleaseActionInput!) {
  forceGrabPendingRelease(input: $input)
}`;

export const dismissPendingReleaseMutation = `mutation DismissPendingRelease($input: PendingReleaseActionInput!) {
  dismissPendingRelease(input: $input)
}`;

// ── Plugins ──────────────────────────────────────────────────────────────

export const refreshPluginRegistryMutation = `mutation RefreshPluginRegistry {
  refreshPluginRegistry {
    id
    name
    description
    version
    latestVersion
    pluginType
    providerType
    author
    official
    publisher
    supportTier
    docsUrl
    sourceRepo
    builtin
    sourceUrl
    sourceKind
    blockedReason
    isInstalled
    isEnabled
    installedVersion
    updateAvailable
    defaultBaseUrl
  }
}`;

export const refreshPluginCatalogMutation = `mutation RefreshPluginCatalog {
  refreshPluginCatalog {
    id
    name
    description
    version
    latestVersion
    pluginType
    providerType
    author
    official
    publisher
    supportTier
    docsUrl
    sourceRepo
    builtin
    sourceUrl
    sourceKind
    blockedReason
    isInstalled
    isEnabled
    installedVersion
    updateAvailable
    installInProgress
    defaultBaseUrl
  }
}`;

export const installPluginMutation = `mutation InstallPlugin($input: InstallPluginInput!) {
  installPlugin(input: $input) {
    id
    pluginId
    name
    description
    version
    sdkVersion
    sdkConstraint
    pluginType
    providerType
    isEnabled
    isBuiltin
    sourceKind
    sourceUrl
    publisher
    supportTier
    docsUrl
    sourceRepo
    manifestUrl
    wasmDigest
    artifactDigest
    installedAt
    updatedAt
  }
}`;

export const beginInstallPluginMutation = `mutation BeginInstallPlugin($input: InstallPluginInput!) {
  beginInstallPlugin(input: $input) {
    pluginId
    operationKind
    state
    label
    stepIndex
    stepCount
    message
    error
  }
}`;

export const uninstallPluginMutation = `mutation UninstallPlugin($input: UninstallPluginInput!) {
  uninstallPlugin(input: $input)
}`;

export const togglePluginMutation = `mutation TogglePlugin($input: TogglePluginInput!) {
  togglePlugin(input: $input) {
    id
    pluginId
    name
    description
    version
    sdkVersion
    sdkConstraint
    pluginType
    providerType
    isEnabled
    isBuiltin
    sourceKind
    sourceUrl
    publisher
    supportTier
    docsUrl
    sourceRepo
    manifestUrl
    wasmDigest
    artifactDigest
    installedAt
    updatedAt
  }
}`;

export const upgradePluginMutation = `mutation UpgradePlugin($input: UpgradePluginInput!) {
  upgradePlugin(input: $input) {
    id
    pluginId
    name
    description
    version
    sdkVersion
    sdkConstraint
    pluginType
    providerType
    isEnabled
    isBuiltin
    sourceKind
    sourceUrl
    publisher
    supportTier
    docsUrl
    sourceRepo
    manifestUrl
    wasmDigest
    artifactDigest
    installedAt
    updatedAt
  }
}`;

export const beginUpgradePluginMutation = `mutation BeginUpgradePlugin($input: UpgradePluginInput!) {
  beginUpgradePlugin(input: $input) {
    pluginId
    operationKind
    state
    label
    stepIndex
    stepCount
    message
    error
  }
}`;

export const inspectManualPluginRepoMutation = `mutation InspectManualPluginRepo($input: ManualPluginRepoInput!) {
  inspectManualPluginRepo(input: $input) {
    githubRepoUrl
    plugin {
      id
      name
      description
      version
      latestVersion
      pluginType
      providerType
      author
      official
      publisher
      supportTier
      docsUrl
      sourceRepo
      builtin
      sourceUrl
      sourceKind
      blockedReason
      isInstalled
      isEnabled
      installedVersion
      updateAvailable
      installInProgress
      defaultBaseUrl
    }
  }
}`;

export const installManualPluginMutation = `mutation InstallManualPlugin($input: ManualPluginRepoInput!) {
  installManualPlugin(input: $input) {
    id
    pluginId
    name
    description
    version
    sdkVersion
    sdkConstraint
    pluginType
    providerType
    isEnabled
    isBuiltin
    sourceKind
    sourceUrl
    publisher
    supportTier
    docsUrl
    sourceRepo
    manifestUrl
    wasmDigest
    artifactDigest
    installedAt
    updatedAt
  }
}`;

// ── Recycle Bin ─────────────────────────────────────────────────────────

export const restoreRecycledItemMutation = `mutation RestoreRecycledItem($id: String!) {
  restoreRecycledItem(id: $id)
}`;

export const deleteRecycledItemMutation = `mutation DeleteRecycledItem($id: String!) {
  deleteRecycledItem(id: $id)
}`;

export const emptyRecycleBinMutation = `mutation EmptyRecycleBin {
  emptyRecycleBin
}`;

// ── Notifications ────────────────────────────────────────────────────────

export const createNotificationChannelMutation = `mutation CreateNotificationChannel($input: CreateNotificationChannelInput!) {
  createNotificationChannel(input: $input) {
    id
    name
    channelType
    configJson
    isEnabled
    createdAt
    updatedAt
  }
}`;

export const updateNotificationChannelMutation = `mutation UpdateNotificationChannel($input: UpdateNotificationChannelInput!) {
  updateNotificationChannel(input: $input) {
    id
    name
    channelType
    configJson
    isEnabled
    createdAt
    updatedAt
  }
}`;

export const deleteNotificationChannelMutation = `mutation DeleteNotificationChannel($id: String!) {
  deleteNotificationChannel(id: $id)
}`;

export const testNotificationChannelMutation = `mutation TestNotificationChannel($id: String!) {
  testNotificationChannel(id: $id)
}`;

export const createNotificationSubscriptionMutation = `mutation CreateNotificationSubscription($input: CreateNotificationSubscriptionInput!) {
  createNotificationSubscription(input: $input) {
    id
    channelId
    eventType
    scope
    scopeId
    isEnabled
    createdAt
    updatedAt
  }
}`;

export const updateNotificationSubscriptionMutation = `mutation UpdateNotificationSubscription($input: UpdateNotificationSubscriptionInput!) {
  updateNotificationSubscription(input: $input) {
    id
    channelId
    eventType
    scope
    scopeId
    isEnabled
    createdAt
    updatedAt
  }
}`;

export const deleteNotificationSubscriptionMutation = `mutation DeleteNotificationSubscription($id: String!) {
  deleteNotificationSubscription(id: $id)
}`;

// ── Rule Sets ────────────────────────────────────────────────────────────

export const createRuleSetMutation = `mutation CreateRuleSet($input: CreateRuleSetInput!) {
  createRuleSet(input: $input) {
    id
    name
    description
    regoSource
    enabled
    priority
    appliedFacets
    isManaged
    managedKey
    createdAt
    updatedAt
  }
}`;

export const updateRuleSetMutation = `mutation UpdateRuleSet($input: UpdateRuleSetInput!) {
  updateRuleSet(input: $input) {
    id
    name
    description
    regoSource
    enabled
    priority
    appliedFacets
    isManaged
    managedKey
    createdAt
    updatedAt
  }
}`;

export const deleteRuleSetMutation = `mutation DeleteRuleSet($id: String!) {
  deleteRuleSet(id: $id)
}`;

export const toggleRuleSetMutation = `mutation ToggleRuleSet($input: ToggleRuleSetInput!) {
  toggleRuleSet(input: $input) {
    id
    name
    description
    regoSource
    enabled
    priority
    appliedFacets
    isManaged
    managedKey
    createdAt
    updatedAt
  }
}`;

export const validateRuleSetMutation = `mutation ValidateRuleSet($input: ValidateRuleSetInput!) {
  validateRuleSet(input: $input) {
    valid
    errors
  }
}`;

export const setTitleRequiredAudioMutation = `mutation SetTitleRequiredAudio($input: SetTitleRequiredAudioInput!) {
  setTitleRequiredAudio(input: $input)
}`;

// ── Setup Wizard ──────────────────────────────────────────────────────

export const completeSetupMutation = `mutation CompleteSetup {
  completeSetup
}`;

// ── External Import (Sonarr/Radarr) ──────────────────────────────────

export const previewExternalImportMutation = `mutation PreviewExternalImport($input: PreviewExternalImportInput!) {
  previewExternalImport(input: $input) {
    sonarrConnected
    radarrConnected
    sonarrVersion
    radarrVersion
    rootFolders { source path }
    downloadClients {
      sources name implementation scryerClientType
      host port useSsl urlBase username apiKey
      dedupKey supported
    }
    indexers {
      sources name implementation scryerProviderType
      baseUrl apiKey dedupKey supported
    }
  }
}`;

export const executeExternalImportMutation = `mutation ExecuteExternalImport($input: ExecuteExternalImportInput!) {
  executeExternalImport(input: $input) {
    mediaPathsSaved
    downloadClientsCreated
    indexersCreated
    pluginsInstalled
    errors
  }
}`;

export const rehydrateAllMetadataMutation = `mutation RehydrateAllMetadata($language: String!) {
  rehydrateAllMetadata(language: $language)
}`;

const ppScriptFields = `
    id name description scriptType scriptContent appliedFacets
    executionMode timeoutSecs priority enabled debug createdAt updatedAt
`;

export const createPostProcessingScriptMutation = `mutation CreatePostProcessingScript($input: CreatePostProcessingScriptInput!) {
  createPostProcessingScript(input: $input) {${ppScriptFields}}
}`;

export const updatePostProcessingScriptMutation = `mutation UpdatePostProcessingScript($input: UpdatePostProcessingScriptInput!) {
  updatePostProcessingScript(input: $input) {${ppScriptFields}}
}`;

export const deletePostProcessingScriptMutation = `mutation DeletePostProcessingScript($id: String!) {
  deletePostProcessingScript(id: $id)
}`;

export const togglePostProcessingScriptMutation = `mutation TogglePostProcessingScript($id: String!) {
  togglePostProcessingScript(id: $id) {${ppScriptFields}}
}`;

// Input type companion — keep in sync with ExecuteExternalImportInput on the backend.
export type DownloadClientApiKeyOverride = {
  dedupKey: string;
  apiKey: string;
};

export type IndexerApiKeyOverride = {
  dedupKey: string;
  apiKey: string;
};

// ── Subtitle mutations ──────────────────────────────────────────────────────

export const searchSubtitlesMutation = `mutation SearchSubtitles($input: SearchSubtitlesInput!) {
  searchSubtitles(input: $input) {
    provider
    providerFileId
    language
    releaseInfo
    score
    hearingImpaired
    forced
    aiTranslated
    machineTranslated
    uploader
    downloadCount
    hashMatched
  }
}`;

export const downloadSubtitleMutation = `mutation DownloadSubtitle($input: DownloadSubtitleInput!) {
  downloadSubtitle(input: $input)
}`;

export const deleteExternalSubtitleMutation = `mutation DeleteExternalSubtitle($input: DeleteExternalSubtitleInput!) {
  deleteExternalSubtitle(input: $input)
}`;

export const blocklistExternalSubtitleMutation = `mutation BlocklistExternalSubtitle($input: BlocklistExternalSubtitleInput!) {
  blocklistExternalSubtitle(input: $input)
}`;

export const clearTitleReleaseBlocklistEntryMutation = `mutation ClearTitleReleaseBlocklistEntry($input: ClearTitleReleaseBlocklistEntryInput!) {
  clearTitleReleaseBlocklistEntry(input: $input)
}`;

// ── Import retry mutations ────────────────────────────────────────────────

export const retryImportMutation = `mutation RetryImport($input: RetryImportInput!) {
  retryImport(input: $input) {
    importId
    decision
    skipReason
    titleId
    sourcePath
    destPath
    errorMessage
  }
}`;

export const ignoreTrackedDownloadMutation = `mutation IgnoreTrackedDownload($input: IgnoreTrackedDownloadInput!) {
  ignoreTrackedDownload(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    removed
    queueItem {
      id
      titleId
      titleName
      clientId
      clientType
      downloadClientItemId
      state
      trackedState
      trackedStatus
    }
  }
}`;

export const assignTrackedDownloadTitleMutation = `mutation AssignTrackedDownloadTitle($input: AssignTrackedDownloadTitleInput!) {
  assignTrackedDownloadTitle(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    removed
    queueItem {
      id
      titleId
      titleName
      facet
      clientId
      clientType
      downloadClientItemId
      state
      trackedState
      trackedStatus
    }
  }
}`;

export type SubtitleSearchResult = {
  provider: string;
  providerFileId: string;
  language: string;
  releaseInfo: string | null;
  score: number;
  hearingImpaired: boolean;
  forced: boolean;
  aiTranslated: boolean;
  machineTranslated: boolean;
  uploader: string | null;
  downloadCount: number | null;
  hashMatched: boolean;
};
