const cheerio = require('cheerio')
const axios = require('axios')
const dayjs = require('dayjs')
const utc = require('dayjs/plugin/utc')
const customParseFormat = require('dayjs/plugin/customParseFormat')
const uniqBy = require('lodash.uniqby')

dayjs.extend(utc)
dayjs.extend(customParseFormat)

const headers = {
  'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36',
  'Accept': 'text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8',
  'Accept-Language': 'hu-HU,hu;q=0.9,en-US;q=0.8,en;q=0.7',
  'Referer': 'https://musor.tv/',
  'Origin': 'https://musor.tv'
}

module.exports = {
  site: 'musor.tv',
  days: 2,
  request: { headers },
  url({ channel, date }) {
    return dayjs.utc().isSame(date, 'd')
      ? `https://musor.tv/mai/tvmusor/${channel.site_id}`
      : `https://musor.tv/napi/tvmusor/${channel.site_id}/${date.format('YYYY.MM.DD')}`
  },
  parser({ content }) {
    const programs = []
    if (!content) return programs

    const [$, items] = parseItems(content)
    items.forEach(item => {
      const prev = programs[programs.length - 1]
      const $item =$(item)
      let start = parseStart($item)
      if (!start) return

      if (prev) prev.stop = start
      const stop = start.add(30, 'm')
      const details = parseDetails($item)
      programs.push({
        title: parseTitle($item),
        subTitle: details.subTitle,
        description: details.description,
        image: parseImage($item),
        start,
        stop
      })
    })

    return programs
  },
  async channels() {
    const html = await axios
      .get('https://musor.tv/', { headers })
      .then(r => r.data)
      .catch(console.log)

    if (!html) return []

    const $ = cheerio.load(html)
    const channels = $('select[name="channelList"] option').toArray()

    return uniqBy(
      channels
        .map(item => {
          const $item =$(item)
          const site_id = $item.attr('value')
          if (!site_id || site_id === '') return null
          return {
            lang: 'hu',
            site_id,
            name: $item.text().trim()
          }
        })
        .filter(Boolean),
      'site_id'
    )
  }
}

function parseImage($item) {
  const imgSrc = $item.find('.prog_entry_screenshot img, .progentry_screenshot img').attr('src')

  if (!imgSrc) return null
  return imgSrc.startsWith('http') ? imgSrc : `https://musor.tv${imgSrc.startsWith('/') ? '' : '/'}${imgSrc}`
}

function parseTitle($item) {
  let title = $item.find('h3.prog_entry_title a, h3.prog_entry_title, h3 > a').text().trim()
  if (!title) {
    title = $item.find('.small_prog_entry_title a, .small_prog_entry_title').text().trim()
  }
  return title
}

function parseDetails($item) {
  const subTitle = $item.find('.prog_entry_short, .small_prog_entry_short').text().trim() || undefined
  const longDesc = $item.find('div.progentrylong').text().trim()

  return {
    subTitle,
    description: longDesc || undefined
  }
}

function parseStart($item) {
  let datetime = $item.find('time').attr('datetime') || $item.find('time').attr('content')
  if (datetime) {
    return dayjs.utc(datetime.replace('GMT', 'T'))
  }

  // Ha nincs time elem, megpróbáljuk kinyerni a szöveges időpontot (pl. " 13:50")
  const timeText = $item.find('.prog_entry_time, .small_prog_entry_time, time').text().trim()
  if (timeText) {
    const match = timeText.match(/(\d{1,2}:\d{2})/)
    if (match) {
      const now = dayjs.utc()
      const [hours, minutes] = match[1].split(':')
      return now.hour(parseInt(hours, 10)).minute(parseInt(minutes, 10)).second(0).millisecond(0)
    }
  }

  return null
}

function parseItems(content) {
  const $ = cheerio.load(content)
  let items = $('div.prog_entry_card').toArray()

  if (!items.length) {
    items = $('div.progarea > div.progentry > div.progentry_internal').toArray()
  }

  return [$, items]
}