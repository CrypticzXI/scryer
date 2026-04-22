export const TITLE_CORE_FIELDS = `
    id
    name
    facet
    monitored
    tags
    externalIds {
      source
      value
    }
    year
    overview
    posterUrl
    posterSourceUrl
    bannerUrl
    bannerSourceUrl
    backgroundUrl
    backgroundSourceUrl
    sortTitle
    slug
    imdbId
    runtimeMinutes
    genres
    contentStatus
    language
    firstAired
    network
    studio
    country
    aliases
    metadataLanguage
    metadataFetchedAt
    qualityProfileId
    requiredAudioLanguagesOverride
    effectiveRequiredAudioLanguages
    inheritsRequiredAudioLanguages
    rootFolderPath
    monitorType
    useSeasonFolders
    monitorSpecials
    interSeasonMovies
    fillerPolicy
    recapPolicy
    createdAt`;

const INTERSTITIAL_MOVIE_FIELDS = `
      tvdbId
      name
      slug
      year
      contentStatus
      overview
      posterUrl
      language
      runtimeMinutes
      sortTitle
      imdbId
      genres
      studio
      digitalReleaseDate
      associationConfidence
      continuityStatus
      movieForm
      confidence
      signalSummary
      placement
      movieTmdbId
      movieMalId`;

const COLLECTION_EPISODE_FIELDS = `
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
      isRecap
      absoluteNumber
      monitored
      createdAt`;

const TITLE_COLLECTION_FIELDS = `
      id
      titleId
      collectionType
      collectionIndex
      label
      orderedPath
      narrativeOrder
      fileSizeBytes
      firstEpisodeNumber
      lastEpisodeNumber
      interstitialMovie {${INTERSTITIAL_MOVIE_FIELDS}
      }
      specialsMovies {${INTERSTITIAL_MOVIE_FIELDS}
      }
      interstitialSeasonEpisode
      monitored
      createdAt
      episodes {${COLLECTION_EPISODE_FIELDS}
      }`;

const TITLE_MEDIA_FILE_FIELDS = `
      id
      titleId
      episodeId
      filePath
      sizeBytes
      qualityLabel
      scanStatus
      createdAt
      videoCodec
      videoWidth
      videoHeight
      videoBitrateKbps
      videoBitDepth
      videoHdrFormat
      videoFrameRate
      videoProfile
      audioCodec
      audioChannels
      audioBitrateKbps
      audioLanguages
      audioStreams {
        codec
        channels
        language
        bitrateKbps
      }
      subtitleLanguages
      subtitleCodecs
      subtitleStreams {
        codec
        language
        name
        forced
        default
      }
      hasMultiaudio
      durationSeconds
      numChapters
      containerFormat
      sceneName
      releaseGroup
      sourceType
      resolution
      videoCodecParsed
      audioCodecParsed
      acquisitionScore
      scoringLog
      indexerSource
      grabbedReleaseTitle
      grabbedAt
      edition
      originalFilePath
      releaseHash`;

const WANTED_ITEM_FIELDS = `
      id
      titleId
      titleName
      episodeId
      collectionId
      mediaType
      searchPhase
      nextSearchAt
      lastSearchAt
      searchCount
      baselineDate
      status
      grabbedRelease
      currentScore
      createdAt
      updatedAt`;

const DOWNLOAD_QUEUE_ITEM_FIELDS = `
    id
    titleId
    titleName
    facet
    isScryerOrigin
    clientId
    clientName
    clientType
    state
    displayState
    progressPercent
    sizeBytes
    remainingSeconds
    queuedAt
    lastUpdatedAt
    attentionRequired
    attentionReason
    downloadClientItemId
    importStatus
    importErrorCode
    importErrorMessage
    importedAt
    deleteStatus
    deleteErrorMessage
    trackedState
    trackedStatus
    trackedStatusMessages
    trackedMatchType`;

  const TITLE_OVERVIEW_FIELDS = `${TITLE_CORE_FIELDS}
    collections {${TITLE_COLLECTION_FIELDS}
    }
    mediaFiles {${TITLE_MEDIA_FILE_FIELDS}
    }
    wantedItems {${WANTED_ITEM_FIELDS}
    }`;

  const TITLE_EVENT_FIELDS = `
    id
    titleId
    episodeId
    collectionId
    eventType
    sourceTitle
    quality
    downloadId
    dataJson
    occurredAt
    createdAt`;

  const TITLE_RELEASE_BLOCKLIST_FIELDS = `
    sourceHint
    sourceTitle
    errorMessage
    attemptedAt`;

const SUBTITLE_DOWNLOAD_FIELDS = `
    id
    mediaFileId
    language
    provider
    filePath
    score
    hearingImpaired
    forced
    aiTranslated
    machineTranslated
    uploader
    releaseInfo
    synced
    downloadedAt`;

  const IMPORT_HISTORY_FIELDS = `
    id
    sourceSystem
    sourceRef
    sourceTitle
    importType
    status
    errorMessage
    decision
    skipReason
    titleId
    sourcePath
    destPath
    startedAt
    finishedAt
    createdAt`;

const PROVIDER_TYPE_FIELDS = `
    providerType
    name
    defaultBaseUrl
    configFields {
      key
      label
      fieldType
      required
      defaultValue
      options { value label }
      helpText
    }`;

const NOTIFICATION_CHANNEL_FIELDS = `
    id
    name
    channelType
    configJson
    isEnabled
    createdAt
    updatedAt`;

const NOTIFICATION_SUBSCRIPTION_FIELDS = `
    id
    channelId
    eventType
    scope
    scopeId
    isEnabled
    createdAt
    updatedAt`;

const DELETE_PREVIEW_FIELDS = `
    fingerprint
    totalFileCount
    mediaCount
    subtitleCount
    imageCount
    otherCount
    directoryCount
    requiresTypedConfirmation
    typedConfirmationPrompt
    targetLabel
    samplePaths`;

export const titleDetailQuery = `query TitleDetail($id: String!) {
  title(id: $id) {${TITLE_CORE_FIELDS}
    collections {${TITLE_COLLECTION_FIELDS}
    }
  }
  titleEvents(titleId: $id, limit: 50, offset: 0) {${TITLE_EVENT_FIELDS}
  }
}`;

export const titleBySlugQuery = `query TitleBySlug($facet: MediaFacetValue!, $slug: String!) {
  titleBySlug(facet: $facet, slug: $slug) {
    id
    slug
  }
}`;

export const titleReleaseBlocklistQuery = `query TitleReleaseBlocklist($titleId: String!, $limit: Int) {
  titleReleaseBlocklist(titleId: $titleId, limit: $limit) {${TITLE_RELEASE_BLOCKLIST_FIELDS}
  }
}`;

export const titleOverviewInitQuery = `query TitleOverviewInit($id: String!, $blocklistLimit: Int) {
  title(id: $id) {${TITLE_OVERVIEW_FIELDS}
  }
  titleEvents(titleId: $id, limit: 50, offset: 0) {${TITLE_EVENT_FIELDS}
  }
  titleReleaseBlocklist(titleId: $id, limit: $blocklistLimit) {${TITLE_RELEASE_BLOCKLIST_FIELDS}
  }
  subtitleDownloads(titleId: $id) {${SUBTITLE_DOWNLOAD_FIELDS}
  }
  setupStatus {
    hasDownloadClients
  }
}`;

export const titleDownloadQueueItemsQuery = `query TitleDownloadQueueItems($id: String!) {
  title(id: $id) {
    id
    downloadQueueItems {${DOWNLOAD_QUEUE_ITEM_FIELDS}
    }
  }
}`;

export const deleteTitlePreviewQuery = `query DeleteTitlePreview($input: DeleteTitlePreviewInput!) {
  deleteTitlePreview(input: $input) {${DELETE_PREVIEW_FIELDS}
  }
}`;

export const deleteMediaFilePreviewQuery = `query DeleteMediaFilePreview($input: DeleteMediaFilePreviewInput!) {
  deleteMediaFilePreview(input: $input) {${DELETE_PREVIEW_FIELDS}
  }
}`;

export const deleteSubtitlePreviewQuery = `query DeleteSubtitlePreview($input: DeleteSubtitlePreviewInput!) {
  deleteSubtitlePreview(input: $input) {${DELETE_PREVIEW_FIELDS}
  }
}`;

export const searchQuery = `query SearchIndexers($query: String!, $imdbId: String, $tvdbId: String, $category: String, $limit: Int) {
  searchReleases(input: {
    query: $query,
    imdbId: $imdbId,
    tvdbId: $tvdbId,
    category: $category,
    limit: $limit
  }) {
    source
    title
    link
    downloadUrl
    sourceKind
    sizeBytes
    publishedAt
    thumbsUp
    thumbsDown
    parsedRelease {
      rawTitle
      normalizedTitle
      releaseGroup
      quality
      source
      videoCodec
      videoEncoding
      audio
      isDualAudio
      isAtmos
      isDolbyVision
      detectedHdr
      parseConfidence
      isProperUpload
      isRemux
      isBdDisk
      isAiEnhanced
    }
    qualityProfileDecision {
      allowed
      blockCodes
      releaseScore
      preferenceScore
      scoringLog {
        code
        delta
        source
        ruleSetName
      }
    }
    seeders
    peers
    infoHash
    freeleech
    downloadVolumeFactor
  }
}`;

export const searchSeriesEpisodeQuery = `query SearchIndexersEpisode($title: String!, $season: String!, $episode: String!, $imdbId: String, $tvdbId: String, $anidbId: String, $category: String, $absoluteEpisode: Int) {
  searchReleases(input: {
    query: $title,
    season: $season,
    episode: $episode,
    imdbId: $imdbId,
    tvdbId: $tvdbId,
    anidbId: $anidbId,
    category: $category,
    absoluteEpisode: $absoluteEpisode
  }) {
    source
    title
    link
    downloadUrl
    sourceKind
    sizeBytes
    publishedAt
    thumbsUp
    thumbsDown
    parsedRelease {
      rawTitle
      normalizedTitle
      releaseGroup
      quality
      source
      videoCodec
      videoEncoding
      audio
      isDualAudio
      isAtmos
      isDolbyVision
      detectedHdr
      parseConfidence
      isProperUpload
      isRemux
      isBdDisk
      isAiEnhanced
    }
    qualityProfileDecision {
      allowed
      blockCodes
      releaseScore
      preferenceScore
      scoringLog {
        code
        delta
        source
        ruleSetName
      }
    }
    seeders
    peers
    infoHash
    freeleech
    downloadVolumeFactor
  }
}`;

export const searchForTitleQuery = `query SearchIndexersForTitle($titleId: String!) {
  searchReleases(input: { titleId: $titleId }) {
    source
    title
    link
    downloadUrl
    sourceKind
    sizeBytes
    publishedAt
    thumbsUp
    thumbsDown
    parsedRelease {
      rawTitle
      normalizedTitle
      releaseGroup
      quality
      source
      videoCodec
      videoEncoding
      audio
      isDualAudio
      isAtmos
      isDolbyVision
      detectedHdr
      parseConfidence
      isProperUpload
      isRemux
      isBdDisk
      isAiEnhanced
    }
    qualityProfileDecision {
      allowed
      blockCodes
      releaseScore
      preferenceScore
      scoringLog {
        code
        delta
        source
        ruleSetName
      }
    }
    seeders
    peers
    infoHash
    freeleech
    downloadVolumeFactor
  }
}`;

export const searchForEpisodeQuery = `query SearchIndexersForEpisode($titleId: String!, $season: String!, $episode: String!) {
  searchReleases(input: {
    titleId: $titleId,
    season: $season,
    episode: $episode
  }) {
    source
    title
    link
    downloadUrl
    sourceKind
    sizeBytes
    publishedAt
    thumbsUp
    thumbsDown
    parsedRelease {
      rawTitle
      normalizedTitle
      releaseGroup
      quality
      source
      videoCodec
      videoEncoding
      audio
      isDualAudio
      isAtmos
      isDolbyVision
      detectedHdr
      parseConfidence
      isProperUpload
      isRemux
      isBdDisk
      isAiEnhanced
    }
    qualityProfileDecision {
      allowed
      blockCodes
      releaseScore
      preferenceScore
      scoringLog {
        code
        delta
        source
        ruleSetName
      }
    }
    seeders
    peers
    infoHash
    freeleech
    downloadVolumeFactor
  }
}`;

export const TITLE_LIST_FIELDS = `
    id
    name
    facet
    monitored
    tags
    imdbId
    posterUrl
    posterSourceUrl
    qualityTier
    sizeBytes
    episodesOwned
    episodesMonitored
    episodesTotal
    contentStatus
    externalIds {
      source
      value
    }`;

export const titlesQuery = `query Titles($facet: MediaFacetValue, $query: String) {
  titles(facet: $facet, query: $query) {
${TITLE_LIST_FIELDS}
  }
}`;

export const titlesByExternalIdsQuery = `query TitlesByExternalIds($source: String!, $values: [String!]!) {
  titlesByExternalIds(source: $source, values: $values) {
${TITLE_LIST_FIELDS}
  }
}`;

export const titleListEntryQuery = `query TitleListEntry($id: String!) {
  title(id: $id) {
${TITLE_LIST_FIELDS}
  }
}`;

type ReactiveRefreshVariableValue = string | number | null;

export type ReactiveRefreshQueryActionInput =
  | {
      key: string;
      kind: "catalogTitles";
      facet?: string | null;
    }
  | {
      key: string;
      kind: "catalogTitle";
      titleId: string;
    }
  | {
      key: string;
      kind: "titleOverview";
      titleId: string;
      blocklistLimit: number;
    }
  | {
      key: string;
      kind: "importHistory";
      limit?: number | null;
    };

export type ReactiveRefreshQueryActionPlan =
  | {
      key: string;
      kind: "catalogTitles";
      titlesAlias: string;
    }
  | {
      key: string;
      kind: "catalogTitle";
      titleAlias: string;
    }
  | {
      key: string;
      kind: "titleOverview";
      titleAlias: string;
      titleEventsAlias: string;
      titleReleaseBlocklistAlias: string;
      subtitleDownloadsAlias: string;
      setupStatusAlias: string;
    }
  | {
      key: string;
      kind: "importHistory";
      importHistoryAlias: string;
    };

export function buildReactiveRefreshQuery(
  actions: ReactiveRefreshQueryActionInput[],
) {
  const variableDefinitions: string[] = [];
  const fields: string[] = [];
  const variables: Record<string, ReactiveRefreshVariableValue> = {};
  const actionPlans: ReactiveRefreshQueryActionPlan[] = [];

  actions.forEach((action, index) => {
    switch (action.kind) {
      case "catalogTitles": {
        const titlesAlias = `catalogTitlesAction${index}`;
        const facetVariableName = `catalogTitlesFacet${index}`;
        variableDefinitions.push(`$${facetVariableName}: MediaFacetValue`);
        fields.push(
          `  ${titlesAlias}: titles(facet: $${facetVariableName}) {\n${TITLE_LIST_FIELDS}\n  }`,
        );
        variables[facetVariableName] = action.facet ?? null;
        actionPlans.push({ key: action.key, kind: action.kind, titlesAlias });
        break;
      }
      case "catalogTitle": {
        const titleAlias = `catalogTitleAction${index}`;
        const titleIdVariableName = `catalogTitleId${index}`;
        variableDefinitions.push(`$${titleIdVariableName}: String!`);
        fields.push(
          `  ${titleAlias}: title(id: $${titleIdVariableName}) {\n${TITLE_LIST_FIELDS}\n  }`,
        );
        variables[titleIdVariableName] = action.titleId;
        actionPlans.push({ key: action.key, kind: action.kind, titleAlias });
        break;
      }
      case "titleOverview": {
        const titleIdVariableName = `titleOverviewId${index}`;
        const blocklistLimitVariableName = `titleOverviewBlocklistLimit${index}`;
        const titleAlias = `titleOverviewTitleAction${index}`;
        const titleEventsAlias = `titleOverviewEventsAction${index}`;
        const titleReleaseBlocklistAlias =
          `titleOverviewBlocklistAction${index}`;
        const subtitleDownloadsAlias =
          `titleOverviewSubtitleDownloadsAction${index}`;
        const setupStatusAlias = `titleOverviewSetupStatusAction${index}`;

        variableDefinitions.push(`$${titleIdVariableName}: String!`);
        variableDefinitions.push(`$${blocklistLimitVariableName}: Int`);
        fields.push(
          `  ${titleAlias}: title(id: $${titleIdVariableName}) {\n${TITLE_OVERVIEW_FIELDS}\n  }`,
        );
        fields.push(
          `  ${titleEventsAlias}: titleEvents(titleId: $${titleIdVariableName}, limit: 50, offset: 0) {\n${TITLE_EVENT_FIELDS}\n  }`,
        );
        fields.push(
          `  ${titleReleaseBlocklistAlias}: titleReleaseBlocklist(titleId: $${titleIdVariableName}, limit: $${blocklistLimitVariableName}) {\n${TITLE_RELEASE_BLOCKLIST_FIELDS}\n  }`,
        );
        fields.push(
          `  ${subtitleDownloadsAlias}: subtitleDownloads(titleId: $${titleIdVariableName}) {\n${SUBTITLE_DOWNLOAD_FIELDS}\n  }`,
        );
        fields.push(
          `  ${setupStatusAlias}: setupStatus {\n    hasDownloadClients\n  }`,
        );
        variables[titleIdVariableName] = action.titleId;
        variables[blocklistLimitVariableName] = action.blocklistLimit;
        actionPlans.push({
          key: action.key,
          kind: action.kind,
          titleAlias,
          titleEventsAlias,
          titleReleaseBlocklistAlias,
          subtitleDownloadsAlias,
          setupStatusAlias,
        });
        break;
      }
      case "importHistory": {
        const importHistoryAlias = `importHistoryAction${index}`;
        const limitVariableName = `importHistoryLimit${index}`;
        variableDefinitions.push(`$${limitVariableName}: Int`);
        fields.push(
          `  ${importHistoryAlias}: importHistory(limit: $${limitVariableName}) {\n${IMPORT_HISTORY_FIELDS}\n  }`,
        );
        variables[limitVariableName] = action.limit ?? null;
        actionPlans.push({
          key: action.key,
          kind: action.kind,
          importHistoryAlias,
        });
        break;
      }
      default: {
        const exhaustiveCheck: never = action;
        throw new Error(`unsupported reactive refresh action: ${exhaustiveCheck}`);
      }
    }
  });

  if (fields.length === 0) {
    throw new Error("reactive refresh query requires at least one action");
  }

  const signature = variableDefinitions.length
    ? `(${variableDefinitions.join(", ")})`
    : "";

  return {
    query: `query ReactiveRefresh${signature} {\n${fields.join("\n")}\n}`,
    variables,
    actionPlans,
  };
}

export const mediaRenamePreviewQuery = `query MediaRenamePreview($input: MediaRenamePreviewInput!) {
  mediaRenamePreview(input: $input) {
    facet
    titleId
    template
    collisionPolicy
    missingMetadataPolicy
    fingerprint
    total
    renamable
    noop
    conflicts
    errors
    items {
      collectionId
      currentPath
      proposedPath
      normalizedFilename
      collision
      reasonCode
      writeAction
      sourceSizeBytes
      sourceMtimeUnixMs
    }
  }
}`;

export const activityQuery = `query Activity($limit: Int, $offset: Int) {
  activityEvents(limit: $limit, offset: $offset) {
    id
    kind
    severity
    channels
    message
    actorUserId
    titleId
    occurredAt
  }
}`;

export const activitySubscriptionQuery = `subscription ActivityStream {
  activityEvents {
    id
    kind
    severity
    channels
    actorUserId
    titleId
    facet
    message
    occurredAt
  }
}`;

const DOMAIN_EVENT_ENVELOPE_FIELDS = `
    sequence
    eventId
    occurredAt
    actorUserId
    titleId
    facet
    eventType
    streamKind
    streamId
    payloadJson`;

export const libraryScanDomainEventsQuery = `query LibraryScanDomainEvents($afterSequence: Int, $limit: Int) {
  domainEvents(
    eventTypes: [library_scan_started, library_scan_title_discovered, library_scan_progressed, library_scan_completed, library_scan_canceled, library_scan_failed]
    afterSequence: $afterSequence
    limit: $limit
  ) {
${DOMAIN_EVENT_ENVELOPE_FIELDS}
  }
}`;

export const libraryScanDomainEventFeedSubscriptionQuery = `subscription LibraryScanDomainEventFeed($afterSequence: Int) {
  domainEventFeed(afterSequence: $afterSequence) {
${DOMAIN_EVENT_ENVELOPE_FIELDS}
  }
}`;

export const jobsQuery = `query Jobs {
  jobs {
    key
    displayName
    description
    category
    section
    manualTriggerAllowed
    usesLibraryScanProgress
    schedule {
      kind
      description
      intervalSeconds
      initialDelaySeconds
      nextRunAt
    }
  }
}`;

export const JOB_RUN_FIELDS = `
  id
  jobKey
  displayName
  category
  section
  status
  triggerSource
  startedAt
  completedAt
  summaryJson
  summaryText
  errorText
  progressJson
  libraryScanProgress {
    sessionId
    facet
    mode
    status
    startedAt
    updatedAt
    foundTitles
    titleMatchTotalKnown
    titleMatchProgress {
      total
      completed
      failed
    }
    hydrationTotalKnown
    hydrationProgress {
      total
      completed
      failed
    }
    mediaAnalysisTotalKnown
    mediaAnalysisProgress {
      total
      completed
      failed
    }
    summary {
      scanned
      matched
      imported
      skipped
      unmatched
    }
  }
`;

export const activeJobRunsQuery = `query ActiveJobRuns {
  activeJobRuns {
${JOB_RUN_FIELDS}
  }
}`;

export const jobRunsQuery = `query JobRuns($jobKey: JobKeyValue!, $limit: Int) {
  jobRuns(jobKey: $jobKey, limit: $limit) {
${JOB_RUN_FIELDS}
  }
}`;

export const recentJobRunsQuery = `query RecentJobRuns($limit: Int) {
  recentJobRuns(limit: $limit) {
${JOB_RUN_FIELDS}
  }
}`;

export const jobRunEventsSubscription = `subscription JobRunEvents {
  jobRunEvents {
${JOB_RUN_FIELDS}
  }
}`;

export const usersQuery = `query Users {
  users {
    id
    username
    entitlements
  }
}`;

export const indexersQuery = `query Indexers($providerType: String) {
  indexers(providerType: $providerType) {
    id
    name
    providerType
    baseUrl
    hasApiKey
    rateLimitSeconds
    rateLimitBurst
    disabledUntil
    isEnabled
    enableInteractiveSearch
    enableAutoSearch
    lastHealthStatus
    lastErrorAt
    lastQueryAt
    configJson
    createdAt
    updatedAt
  }
}`;

export const indexerProviderTypesQuery = `query IndexerProviderTypes {
  indexerProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
}`;

export const downloadClientProviderTypesQuery = `query DownloadClientProviderTypes {
  downloadClientProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
}`;

export const downloadClientsQuery = `query DownloadClients {
  downloadClientConfigs {
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

export const downloadQueueQuery = `query DownloadQueue($includeAllActivity: Boolean, $includeHistoryOnly: Boolean, $activityFilter: DownloadActivityFilterValue) {
  downloadQueue(includeAllActivity: $includeAllActivity, includeHistoryOnly: $includeHistoryOnly, activityFilter: $activityFilter) {${DOWNLOAD_QUEUE_ITEM_FIELDS}
  }
}`;

export const downloadImportQuery = `query DownloadImport($limit: Int, $offset: Int, $filter: DownloadImportFilterValue) {
  downloadImport(limit: $limit, offset: $offset, filter: $filter) {
    items {${DOWNLOAD_QUEUE_ITEM_FIELDS}
    }
    hasMore
    totalCount
  }
}`;

export const downloadHistoryQuery = `query DownloadHistory($limit: Int, $offset: Int, $filters: [DownloadHistoryFilterValue!], $clientIds: [String!], $scryerSubmittedOnly: Boolean, $sortKey: DownloadHistorySortKeyValue, $sortDirection: SortDirectionValue) {
  downloadHistory(limit: $limit, offset: $offset, filters: $filters, clientIds: $clientIds, scryerSubmittedOnly: $scryerSubmittedOnly, sortKey: $sortKey, sortDirection: $sortDirection) {
    items {${DOWNLOAD_QUEUE_ITEM_FIELDS}
    }
    hasMore
    totalCount
    availableClients {
      clientId
      clientName
      clientType
    }
  }
}`;

export const downloadQueueSubscription = `subscription DownloadQueueStream($includeAllActivity: Boolean, $includeHistoryOnly: Boolean, $activityFilter: DownloadActivityFilterValue) {
  downloadQueue(includeAllActivity: $includeAllActivity, includeHistoryOnly: $includeHistoryOnly, activityFilter: $activityFilter) {${DOWNLOAD_QUEUE_ITEM_FIELDS}
  }
}`;

export const importQueueCountQuery = `query ImportQueueCount {
  downloadImport(limit: 1, offset: 0, filter: all) {
    totalCount
  }
}`;

const downloadClientFieldSelection = `
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
    updatedAt`;

const indexerFieldSelection = `
    id
    name
    providerType
    baseUrl
    hasApiKey
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
    updatedAt`;

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

// Batched query for quality profiles page: 5 requests → 1
export const qualityProfilesInitQuery = `query QualityProfilesInit {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
}`;

export const movieOverviewSettingsInitQuery = `query MovieOverviewSettingsInit {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
  mediaSettings(scope: movie) {${mediaSettingsFieldSelection}
  }
}`;

export const seriesOverviewSettingsInitQuery = `query SeriesOverviewSettingsInit($scope: ContentScopeValue!) {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
  mediaSettings(scope: $scope) {${mediaSettingsFieldSelection}
  }
}`;

export const cutoffUnmetTitlesQuery = `query CutoffUnmetTitles($facet: MediaFacetValue) {
  cutoffUnmetTitles(facet: $facet) {
    id
    name
    facet
    posterUrl
    externalIds {
      source
      value
    }
    currentTier
    targetTier
  }
}`;

export const downloadClientsInitQuery = `query DownloadClientsInit {
  downloadClientConfigs {${downloadClientFieldSelection}
  }
  downloadClientProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
}`;

export const indexersInitQuery = `query IndexersInit($providerType: String) {
  indexers(providerType: $providerType) {${indexerFieldSelection}
  }
  indexerProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
}`;

export const setupWizardProviderTypesInitQuery = `query SetupWizardProviderTypesInit {
  downloadClientProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
  indexerProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
}`;

export const rootFoldersQuery = `query RootFolders($facet: MediaFacetValue!) {
  rootFolders(facet: $facet) { path isDefault }
}`;

export const mediaSettingsInitQuery = `query MediaSettingsInit($scope: ContentScopeValue!) {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
  mediaSettings(scope: $scope) {${mediaSettingsFieldSelection}
  }
}`;

export const globalSearchInitQuery = `query GlobalSearchInit {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
  movieSettings: mediaSettings(scope: movie) {${mediaSettingsFieldSelection}
  }
  seriesSettings: mediaSettings(scope: series) {${mediaSettingsFieldSelection}
  }
  animeSettings: mediaSettings(scope: anime) {${mediaSettingsFieldSelection}
  }
}`;

// Batched query for routing page bootstrap.
export const routingPageInitQuery = `query RoutingPageInit($scopeId: ContentScopeValue!) {
  downloadClientConfigs {${downloadClientFieldSelection}
  }
  indexers {${indexerFieldSelection}
  }
  downloadClientRouting(scope: $scopeId) {${downloadClientRoutingFieldSelection}
  }
  indexerRouting(scope: $scopeId) {${indexerRoutingFieldSelection}
  }
}`;

// TLS settings query
export const tlsSettingsQuery = `query TlsSettings {
  serviceSettings {${serviceSettingsFieldSelection}
  }
}`;

// Acquisition settings query
export const acquisitionSettingsQuery = `query AcquisitionSettings {
  acquisitionSettings {
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

export const generalSettingsQuery = `query GeneralSettings {
  generalSettings {
    keepHistoryForever
    historyRetentionDays
  }
}`;

export const delayProfilesQuery = `query DelayProfiles {
  delayProfiles {
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

export const libraryPathsQuery = `query LibraryPaths {
  libraryPaths {${libraryPathsFieldSelection}
  }
}`;

export const subtitleSettingsQuery = `query SubtitleSettings {
  subtitleSettings {
    enabled
    hasOpenSubtitlesApiKey
    openSubtitlesUsername
    hasOpenSubtitlesPassword
    languages {
      code
      hearingImpaired
      forced
    }
    autoDownloadOnImport
    minimumScoreSeries
    minimumScoreMovie
    searchIntervalHours
    includeAiTranslated
    includeMachineTranslated
    syncEnabled
    syncThresholdSeries
    syncThresholdMovie
    syncMaxOffsetSeconds
  }
}`;

// Batched query for download client routing: 2 requests → 1
export const downloadClientRoutingInitQuery = `query DownloadClientRoutingInit($scopeId: ContentScopeValue!) {
  downloadClientConfigs {${downloadClientFieldSelection}
  }
  downloadClientRouting(scope: $scopeId) {${downloadClientRoutingFieldSelection}
  }
}`;

// Batched query for indexer routing: 2 requests → 1
export const indexerRoutingInitQuery = `query IndexerRoutingInit($scopeId: ContentScopeValue!) {
  indexers {${indexerFieldSelection}
  }
  indexerRouting(scope: $scopeId) {${indexerRoutingFieldSelection}
  }
}`;

export const meQuery = `query Me {
  me {
    id
    username
    entitlements
  }
}`;

export const importHistoryQuery = `query ImportHistory($limit: Int) {
  importHistory(limit: $limit) {${IMPORT_HISTORY_FIELDS}
  }
}`;

export const importHistoryChangedSubscription = `subscription ImportHistoryChanged {
  importHistoryChanged
}`;

export const settingsChangedSubscription = `subscription SettingsChanged {
  settingsChanged
}`;

export const systemHealthQuery = `query SystemHealth {
  systemHealth {
    serviceReady
    dbPath
    totalTitles
    monitoredTitles
    totalUsers
    titlesMovie
    titlesSeries
    titlesAnime
    titlesOther
    recentEvents
    recentEventPreview
    dbMigrationVersion
    dbPendingMigrations
    smgCertExpiresAt
    smgCertDaysRemaining
    indexerStats {
      indexerId
      indexerName
      queriesLast24H
      successfulLast24H
      failedLast24H
      lastQueryAt
      apiCurrent
      apiMax
      grabCurrent
      grabMax
    }
  }
}`;

export const serviceLogsQuery = `query ServiceLogs($limit: Int) {
  serviceLogs(limit: $limit) {
    generatedAt
    lines
    count
  }
}`;

export const serviceLogLinesSubscription = `subscription ServiceLogLines {
  serviceLogLines
}`;

export const previewManualImportQuery = `query PreviewManualImport($downloadClientItemId: String!, $titleId: String!) {
  previewManualImport(downloadClientItemId: $downloadClientItemId, titleId: $titleId) {
    files {
      filePath
      fileName
      sizeBytes
      quality
      parsedSeason
      parsedEpisodes
      suggestedEpisodeId
      suggestedEpisodeLabel
    }
    availableEpisodes {
      id
      titleId
      collectionId
      episodeType
      episodeNumber
      seasonNumber
      episodeLabel
      title
      monitored
    }
  }
}`;

export const wantedItemsQuery = `query WantedItems($status: WantedStatusValue, $mediaType: WantedMediaTypeValue, $titleId: String, $limit: Int, $offset: Int) {
  wantedItems(status: $status, mediaType: $mediaType, titleId: $titleId, limit: $limit, offset: $offset) {
    items {
      id
      titleId
      titleName
      episodeId
      collectionId
      seasonNumber
      mediaType
      searchPhase
      nextSearchAt
      lastSearchAt
      searchCount
      baselineDate
      status
      grabbedRelease
      currentScore
      createdAt
      updatedAt
    }
    total
  }
}`;

export const releaseDecisionsQuery = `query ReleaseDecisions($wantedItemId: String!, $limit: Int) {
  wantedItem(id: $wantedItemId) {
    id
    releaseDecisions(limit: $limit) {
      id
      wantedItemId
      titleId
      releaseTitle
      releaseUrl
      releaseSizeBytes
      decisionCode
      candidateScore
      currentScore
      scoreDelta
      explanationJson
      createdAt
    }
  }
}`;

export const pluginsQuery = `query Plugins {
  plugins {
    id
    name
    description
    version
    pluginType
    providerType
    author
    official
    builtin
    sourceUrl
    isInstalled
    isEnabled
    installedVersion
    updateAvailable
  }
}`;

export const recycledItemsQuery = `query RecycledItems($limit: Int, $offset: Int) {
  recycledItems(limit: $limit, offset: $offset) {
    items {
      id
      originalPath
      fileName
      sizeBytes
      titleId
      reason
      recycledAt
      mediaRoot
    }
    totalCount
  }
}`;

export const ruleSetsQuery = `query RuleSets {
  ruleSets {
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

// ── Community Rule Packs ──────────────────────────────────────────────

export const rulePackRegistryQuery = `query RulePackRegistry {
  rulePackRegistry {
    id
    name
    description
    author
    version
  }
}`;

export const rulePackTemplatesQuery = `query RulePackTemplates($packId: String!) {
  rulePackTemplates(packId: $packId) {
    id
    title
    description
    category
    regoSource
    appliedFacets
  }
}`;

// ── Notifications ─────────────────────────────────────────────────────

export const notificationChannelsQuery = `query NotificationChannels {
  notificationChannels {${NOTIFICATION_CHANNEL_FIELDS}
  }
}`;

export const notificationSubscriptionsQuery = `query NotificationSubscriptions {
  notificationSubscriptions {${NOTIFICATION_SUBSCRIPTION_FIELDS}
  }
}`;

export const notificationProviderTypesQuery = `query NotificationProviderTypes {
  notificationProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
}`;

export const notificationEventTypesQuery = `query NotificationEventTypes {
  notificationEventTypes
}`;

export const notificationsInitQuery = `query NotificationsInit {
  notificationChannels {${NOTIFICATION_CHANNEL_FIELDS}
  }
  notificationSubscriptions {${NOTIFICATION_SUBSCRIPTION_FIELDS}
  }
  notificationProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
  notificationEventTypes
}`;

// ── Metadata Gateway (proxied through backend) ────────────────────────

const METADATA_SEARCH_FIELDS = `
    tvdbId
    name
    imdbId
    slug
    type
    year
    status
    overview
    popularity
    posterUrl
    language
    runtimeMinutes
    sortTitle`;

export const searchMetadataQuery = `query SearchMetadata($query: String!, $type: String!, $limit: Int, $language: String! = "eng", $year: Int) {
  searchMetadata(query: $query, type: $type, limit: $limit, language: $language, year: $year) {${METADATA_SEARCH_FIELDS}
  }
}`;

export const pendingImportCountsQuery = `query PendingImportCounts {
  pendingImportCounts {
    movie
    series
    anime
  }
}`;

export const pendingImportsQuery = `query PendingImports($facet: MediaFacetValue!, $limit: Int = 50, $offset: Int = 0) {
  pendingImports(facet: $facet, limit: $limit, offset: $offset) {
    total
    items {
      id
      facet
      displayName
      path
      folderPath
      query
      yearHint
      reason
      searchAttempts {
        query
        resultCount
        topResults
        summary
      }
    }
  }
}`;

export const searchMetadataMultiQuery = `query SearchMetadataMulti($query: String!, $limit: Int, $language: String! = "eng") {
  searchMetadataMulti(query: $query, limit: $limit, language: $language) {
    movies {${METADATA_SEARCH_FIELDS}
    }
    series {${METADATA_SEARCH_FIELDS}
    }
    anime {${METADATA_SEARCH_FIELDS}
    }
  }
}`;

export const metadataMovieQuery = `query MetadataMovie($tvdbId: Int!, $language: String! = "eng") {
  metadataMovie(tvdbId: $tvdbId, language: $language) {
    tvdbId
    name
    slug
    year
    status
    overview
    posterUrl
    language
    runtimeMinutes
    sortTitle
    imdbId
    genres
    studio
    tmdbReleaseDate
  }
}`;

export const metadataSeriesQuery = `query MetadataSeries($id: String!, $includeEpisodes: Boolean, $language: String! = "eng") {
  metadataSeries(id: $id, includeEpisodes: $includeEpisodes, language: $language) {
    tvdbId
    name
    sortName
    slug
    year
    status
    firstAired
    overview
    network
    runtimeMinutes
    posterUrl
    country
    genres
    aliases
    seasons {
      tvdbId
      number
      label
      episodeType
    }
    episodes {
      tvdbId
      episodeNumber
      seasonNumber
      name
      aired
      runtimeMinutes
      isFiller
    }
  }
}`;

export const pendingReleasesQuery = `query PendingReleases {
  pendingReleases {
    id
    wantedItemId
    titleId
    releaseTitle
    releaseUrl
    releaseSizeBytes
    releaseScore
    scoringLogJson
    indexerSource
    addedAt
    delayUntil
    status
  }
}`;

export const calendarEpisodesQuery = `query CalendarEpisodes($startDate: String!, $endDate: String!) {
  calendarEpisodes(startDate: $startDate, endDate: $endDate) {
    id
    titleId
    titleName
    titleFacet
    seasonNumber
    episodeNumber
    episodeTitle
    airDate
    monitored
  }
}`;

// ── Setup Wizard ──────────────────────────────────────────────────────

export const setupStatusQuery = `query SetupStatus {
  setupStatus {
    setupComplete
    hasDownloadClients
    hasIndexers
  }
}`;

export const browsePathQuery = `query BrowsePath($path: String!) {
  browsePath(path: $path) {
    name
    path
  }
}`;

export const postProcessingScriptsQuery = `query PostProcessingScripts {
  postProcessingScripts {
    id
    name
    description
    scriptType
    scriptContent
    appliedFacets
    executionMode
    timeoutSecs
    priority
    enabled
    debug
    createdAt
    updatedAt
  }
}`;

export const postProcessingScriptRunsQuery = `query PostProcessingScriptRuns($scriptId: String!, $limit: Int) {
  postProcessingScriptRuns(scriptId: $scriptId, limit: $limit) {
    id
    scriptId
    scriptName
    titleId
    titleName
    facet
    filePath
    status
    exitCode
    stdoutTail
    stderrTail
    durationMs
    startedAt
    completedAt
  }
}`;

export const subtitleDownloadsQuery = `query SubtitleDownloads($titleId: String!) {
  subtitleDownloads(titleId: $titleId) {${SUBTITLE_DOWNLOAD_FIELDS}
  }
}`;

export const titleHistoryQuery = `query TitleHistory($filter: TitleHistoryFilterInput!) {
  titleHistory(filter: $filter) {
    records {
      id
      titleId
      episodeId
      collectionId
      eventType
      sourceTitle
      quality
      downloadId
      dataJson
      occurredAt
      createdAt
    }
    totalCount
  }
}`;

export const episodeHistoryQuery = `query EpisodeHistory($episodeId: String!, $limit: Int) {
  episodeHistory(episodeId: $episodeId, limit: $limit) {
    id
    titleId
    episodeId
    collectionId
    eventType
    sourceTitle
    quality
    downloadId
    dataJson
    occurredAt
    createdAt
  }
}`;
